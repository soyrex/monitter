//! Runtime-only projection of user-managed MCP servers and portable skills.
//!
//! The private extension store is read once for a new native launch. Values in
//! this module are deliberately never added to snapshots or diagnostic text.

use crate::extensions::{ExtensionConfig, McpServerConfig, McpTransport};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Default)]
pub(crate) struct RuntimeExtensions {
    servers: Vec<McpServerConfig>,
    skill_context: String,
}

pub(crate) struct PrivateConfigFile(PathBuf);

impl PrivateConfigFile {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateConfigFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(crate) fn write_private_claude_config(value: &Value) -> Result<PrivateConfigFile, String> {
    use std::fs::OpenOptions;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let directory = std::env::temp_dir();
    for _ in 0..8 {
        let path = directory.join(format!("monitter-claude-mcp-{}.json", crate::model::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&path) {
            Ok(mut file) => {
                if let Err(error) = serde_json::to_writer(&mut file, value) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(format!(
                        "Could not encode private Claude MCP configuration: {error}"
                    ));
                }
                if let Err(error) = file.flush() {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(format!(
                        "Could not save private Claude MCP configuration: {error}"
                    ));
                }
                return Ok(PrivateConfigFile(path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Could not create private Claude MCP configuration: {error}"
                ))
            }
        }
    }
    Err("Could not allocate a private Claude MCP configuration file.".into())
}

impl RuntimeExtensions {
    pub(crate) fn for_agent(config: &ExtensionConfig, agent_id: &str) -> Self {
        let servers = config
            .mcp_servers
            .iter()
            .filter(|server| server.enabled && server.agent_ids.iter().any(|id| id == agent_id))
            .cloned()
            .collect();
        let skills = config
            .skills
            .iter()
            .filter(|skill| skill.enabled && skill.agent_ids.iter().any(|id| id == agent_id));
        let mut skill_context = String::new();
        for skill in skills {
            if !skill_context.is_empty() {
                skill_context.push_str("\n\n");
            }
            skill_context.push_str("<monitter-managed-skill name=");
            skill_context.push_str(
                &serde_json::to_string(&skill.name).unwrap_or_else(|_| "\"skill\"".into()),
            );
            skill_context.push_str(">\n");
            skill_context.push_str(&skill.content);
            skill_context.push_str("\n</monitter-managed-skill>");
        }
        Self {
            servers,
            skill_context,
        }
    }

    pub(crate) fn validate_for(&mut self, provider: &str, host_kind: &str) -> Result<(), String> {
        if self.servers.is_empty() {
            return Ok(());
        }
        if host_kind == "ssh" {
            return Err("Managed MCP servers are not yet supported on SSH hosts because Monitter cannot securely transfer their private invocation values to the remote runtime.".into());
        }
        match provider {
            "codex" | "claude" | "acp" => {}
            "opencode" => return Err("Managed MCP servers are not yet supported by the OpenCode adapter; Monitter will not silently ignore them.".into()),
            other => return Err(format!("Managed MCP servers are not supported by the {other} adapter; Monitter will not silently ignore them.")),
        }
        for server in &mut self.servers {
            validate_id(&server.id)?;
            if server.id == "monitter" {
                return Err(
                    "Managed MCP server id 'monitter' is reserved for Monitter collaboration."
                        .into(),
                );
            }
            match server.transport {
                McpTransport::Stdio if server.command.trim().is_empty() => {
                    return Err(format!(
                        "Managed MCP server '{}' needs a command.",
                        server.name
                    ));
                }
                McpTransport::Http if server.url.trim().is_empty() => {
                    return Err(format!("Managed MCP server '{}' needs a URL.", server.name));
                }
                _ => {}
            }
            if server.transport == McpTransport::Stdio {
                server.command = crate::acp_discovery::resolve_command(&server.command)
                    .map_err(|_| "Managed MCP executable was not found. Install it separately or choose its absolute path.".to_string())?
                    .to_string_lossy().into_owned();
            }
        }
        Ok(())
    }

    pub(crate) fn prompt(&self, prompt: &str) -> String {
        if self.skill_context.is_empty() {
            prompt.into()
        } else {
            format!(
                "Portable skills enabled for this agent:\n{}\n\nUser request:\n{}",
                self.skill_context, prompt
            )
        }
    }

    pub(crate) fn mcp_fingerprint(&self) -> Option<String> {
        if self.servers.is_empty() {
            return None;
        }
        let encoded = serde_json::to_vec(&self.servers).ok()?;
        Some(format!("{:x}", Sha256::digest(encoded)))
    }

    /// Codex app-server's `config` object is sent over its owned stdin pipe.
    pub(crate) fn codex_config(&self) -> Map<String, Value> {
        let mut config = Map::new();
        for server in &self.servers {
            let prefix = format!("mcp_servers.{}", server.id);
            match server.transport {
                McpTransport::Stdio => {
                    config.insert(format!("{prefix}.command"), json!(server.command));
                    config.insert(format!("{prefix}.args"), json!(server.args));
                    if !server.env.is_empty() {
                        config.insert(format!("{prefix}.env"), json!(server.env));
                    }
                }
                McpTransport::Http => {
                    config.insert(format!("{prefix}.url"), json!(server.url));
                    if !server.headers.is_empty() {
                        config.insert(format!("{prefix}.http_headers"), json!(server.headers));
                    }
                }
            }
            config.insert(format!("{prefix}.enabled"), Value::Bool(true));
            // An explicitly assigned server must not disappear behind a
            // discarded provider warning when initialization fails.
            config.insert(format!("{prefix}.required"), Value::Bool(true));
            // Managed servers keep the harness/user approval policy. Only the
            // built-in collaboration server receives an explicit allowlist.
            config.insert(
                format!("{prefix}.default_tools_approval_mode"),
                json!("prompt"),
            );
        }
        config
    }

    pub(crate) fn claude_config(&self) -> Value {
        let mut servers = Map::new();
        for server in &self.servers {
            let value = match server.transport {
                McpTransport::Stdio => json!({
                    "command": server.command,
                    "args": server.args,
                    "env": server.env,
                }),
                McpTransport::Http => json!({
                    "type": "http",
                    "url": server.url,
                    "headers": server.headers,
                }),
            };
            servers.insert(server.id.clone(), value);
        }
        json!({"mcpServers": servers})
    }

    pub(crate) fn has_http(&self) -> bool {
        self.servers
            .iter()
            .any(|server| server.transport == McpTransport::Http)
    }

    pub(crate) fn acp_servers(&self) -> Result<Value, String> {
        let servers = self.servers.iter().map(|server| match server.transport {
            McpTransport::Stdio => {
                let command = crate::acp_discovery::resolve_command(&server.command)
                    .map_err(|_| "Managed MCP executable was not found. Install it separately or choose its absolute path.".to_string())?;
                Ok(json!({
                    "name": server.id, "command": command, "args": server.args,
                    "env": server.env.iter().map(|(name,value)| json!({"name":name,"value":value})).collect::<Vec<_>>()
                }))
            }
            McpTransport::Http => Ok(json!({
                "type":"http", "name":server.id, "url":server.url,
                "headers":server.headers.iter().map(|(name,value)| json!({"name":name,"value":value})).collect::<Vec<_>>()
            })),
        }).collect::<Result<Vec<_>, String>>()?;
        Ok(Value::Array(servers))
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(format!(
            "Managed MCP server id '{id}' may contain only ASCII letters, numbers, '-' and '_'."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::ManagedSkill;
    use std::collections::BTreeMap;

    fn server(agent_ids: Vec<String>) -> McpServerConfig {
        McpServerConfig {
            id: "docs".into(),
            name: "Docs".into(),
            enabled: true,
            agent_ids,
            transport: McpTransport::Stdio,
            command: "private-command".into(),
            args: vec!["private-arg".into()],
            env: BTreeMap::from([("TOKEN".into(), "private-token".into())]),
            url: String::new(),
            headers: BTreeMap::new(),
        }
    }

    #[test]
    fn scopes_empty_to_none_and_exact_agent_to_one() {
        let mut config = ExtensionConfig::default();
        config.mcp_servers.push(server(vec![]));
        assert!(RuntimeExtensions::for_agent(&config, "agent-a")
            .servers
            .is_empty());
        config.mcp_servers.push(server(vec!["agent-a".into()]));
        assert_eq!(
            RuntimeExtensions::for_agent(&config, "agent-a")
                .servers
                .len(),
            1
        );
        assert!(RuntimeExtensions::for_agent(&config, "agent-b")
            .servers
            .is_empty());
    }

    #[test]
    fn skill_text_is_only_added_for_assigned_enabled_skill() {
        let mut config = ExtensionConfig::default();
        config.skills.push(ManagedSkill {
            id: "s".into(),
            name: "Review".into(),
            description: String::new(),
            enabled: true,
            agent_ids: vec!["agent-a".into()],
            content: "Check boundaries.".into(),
        });
        let runtime = RuntimeExtensions::for_agent(&config, "agent-a");
        assert!(runtime.prompt("hello").contains("Check boundaries."));
        assert_eq!(
            RuntimeExtensions::for_agent(&config, "agent-b").prompt("hello"),
            "hello"
        );
    }

    #[test]
    fn codex_payload_keeps_secrets_in_stdin_config_and_approval_prompted() {
        let config = ExtensionConfig {
            revision: None,
            mcp_servers: vec![server(vec!["a".into()])],
            skills: vec![],
        };
        let runtime = RuntimeExtensions::for_agent(&config, "a");
        let payload = runtime.codex_config();
        assert_eq!(payload["mcp_servers.docs.env"]["TOKEN"], "private-token");
        assert_eq!(
            payload["mcp_servers.docs.default_tools_approval_mode"],
            "prompt"
        );
    }

    #[test]
    fn rejects_unsupported_transport_and_ssh_visibly() {
        let config = ExtensionConfig {
            revision: None,
            mcp_servers: vec![server(vec!["a".into()])],
            skills: vec![],
        };
        let mut runtime = RuntimeExtensions::for_agent(&config, "a");
        assert!(runtime
            .validate_for("codex", "ssh")
            .unwrap_err()
            .contains("SSH"));
        assert!(runtime
            .validate_for("opencode", "local")
            .unwrap_err()
            .contains("OpenCode"));
    }

    #[test]
    fn claude_private_file_exposes_only_its_path_and_is_removed_on_drop() {
        let value = json!({"mcpServers":{"docs":{"env":{"TOKEN":"private-token"}}}});
        let file = write_private_claude_config(&value).unwrap();
        let path = file.path().to_owned();
        assert_eq!(fs::read_to_string(&path).unwrap(), value.to_string());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(file);
        assert!(!path.exists());
    }

    #[test]
    fn fingerprint_scopes_secrets_without_returning_them() {
        let config = ExtensionConfig {
            revision: None,
            mcp_servers: vec![server(vec!["a".into()])],
            skills: vec![],
        };
        let fingerprint = RuntimeExtensions::for_agent(&config, "a")
            .mcp_fingerprint()
            .unwrap();
        assert_eq!(fingerprint.len(), 64);
        assert!(!fingerprint.contains("private-token"));
    }
}
