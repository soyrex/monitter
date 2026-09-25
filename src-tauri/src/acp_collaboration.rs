//! Scoped collaboration configuration for ACP sessions.
//!
//! The token travels only in the ACP session's HTTP MCP headers. It is never
//! put in a URL or launch argument, saved in a task, or included in a
//! diagnostic.

use crate::collaboration_transport::SessionGrant;
use serde_json::{Value, json};

pub(crate) fn mcp_servers(grant: Option<&SessionGrant>) -> Value {
    mcp_servers_with_jev(grant, None)
}

pub(crate) fn mcp_servers_with_jev(
    grant: Option<&SessionGrant>,
    jev_decisions: Option<&SessionGrant>,
) -> Value {
    let mut servers = Vec::new();
    if let Some(grant) = grant {
        servers.push(json!({
            "type": "http",
            "name": "monitter",
            "url": grant.endpoint,
            "headers": [{"name": "Authorization", "value": format!("Bearer {}", grant.token)}]
        }));
    }
    if let Some(grant) = jev_decisions {
        servers.push(json!({
            "type": "http",
            "name": "jev_decisions",
            "url": grant.endpoint,
            "headers": [{"name": "Authorization", "value": format!("Bearer {}", grant.token)}]
        }));
    }
    Value::Array(servers)
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

    #[test]
    fn jev_decisions_is_a_distinct_http_server_with_its_own_bearer_grant() {
        let collaboration = SessionGrant {
            endpoint: "http://127.0.0.1:4444/mcp".into(),
            token: "collaboration-token".into(),
        };
        let jev = SessionGrant {
            endpoint: "http://127.0.0.1:4555/mcp".into(),
            token: "jev-token".into(),
        };
        let servers = mcp_servers_with_jev(Some(&collaboration), Some(&jev));
        assert_eq!(servers[0]["name"], "monitter");
        assert_eq!(servers[0]["url"], collaboration.endpoint);
        assert_eq!(
            servers[0]["headers"][0]["value"],
            "Bearer collaboration-token"
        );
        assert_eq!(servers[1]["name"], "jev_decisions");
        assert_eq!(servers[1]["url"], jev.endpoint);
        assert_eq!(servers[1]["headers"][0]["value"], "Bearer jev-token");
    }
}
