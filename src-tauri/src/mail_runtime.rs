//! Task-scoped Mail MCP grants, independent of collaboration or Jev permission.
use crate::{
    collaboration_transport::{Broker, SessionGrant},
    extensions_runtime::RuntimeExtensions,
    model::{AcpLaunch, Agent, Host},
    Service,
};
use std::{
    collections::HashSet,
    path::Path,
    sync::{atomic::Ordering, Arc},
};

fn supported(provider: &str, host_kind: &str, launch: Option<&AcpLaunch>) -> bool {
    host_kind == "local"
        && matches!(provider, "codex" | "claude" | "acp")
        && !launch.is_some_and(|launch| {
            Path::new(&launch.command)
                .file_name()
                .and_then(|name| name.to_str())
                == Some("monitter-agy-acp")
                || launch
                    .args
                    .iter()
                    .any(|arg| arg.ends_with("agy-acp-bridge.mjs"))
        })
}
fn eligible(agent: &Agent, hosts: &[Host]) -> bool {
    !agent.internal
        && hosts
            .iter()
            .find(|host| host.id == agent.host_id)
            .is_some_and(|host| supported(&agent.provider, &host.kind, agent.acp.as_ref()))
}
impl Service {
    pub(crate) fn runtime_extensions_for_task(
        self: &Arc<Self>,
        task_id: &str,
    ) -> Result<RuntimeExtensions, String> {
        let (task, host) = self.task_and_host(task_id)?;
        let mut extensions = self.runtime_extensions_for_agent(&task.agent_id)?;
        extensions.auto_approve_monitter_mcp =
            self.committed_data()?.snapshot.settings.auto_approve_monitter_mcp;
        // Check the user's catalogue before adding our reserved server identity.
        extensions.validate_for(&task.provider, &host.kind)?;
        if let Some(grant) = self.mail_grant(task_id)? {
            extensions.attach_mail(grant);
        }
        Ok(extensions)
    }

    fn mail_caller_agent(&self, task_id: &str) -> Result<String, String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err("Monitter is shutting down.".into());
        }
        let data = self.committed_data()?;
        let task = data
            .snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or("Mail caller task was not found.")?;
        let agent = data
            .snapshot
            .agents
            .iter()
            .find(|agent| agent.id == task.agent_id)
            .ok_or("Mail caller agent was not found.")?;
        let host = data
            .task_hosts
            .get(task_id)
            .ok_or("Mail caller host was not found.")?;
        if task.status != "running"
            || !eligible(agent, &data.snapshot.hosts)
            || !supported(&task.provider, &host.kind, task.acp.as_ref())
        {
            return Err("Mail access requires a running, supported local user agent task.".into());
        }
        Ok(agent.id.clone())
    }

    fn mail_grant(self: &Arc<Self>, task_id: &str) -> Result<Option<SessionGrant>, String> {
        let (task, _) = self.task_and_host(task_id)?;
        if !self.mail.has_agent_access(&task.agent_id)? {
            self.revoke_mail_grant(task_id);
            return Ok(None);
        }
        self.mail_caller_agent(task_id)?;
        let mut grants = self
            .mail_grants
            .lock()
            .map_err(|_| "Mail grant lock failed.")?;
        if let Some(grant) = grants.get(task_id) {
            return Ok(Some(grant.clone()));
        }
        let mut broker = self
            .mail_broker
            .lock()
            .map_err(|_| "Mail broker lock failed.")?;
        if broker.is_none() {
            let weak = Arc::downgrade(self);
            *broker = Some(Broker::start_mail(Arc::new(move |task, tool, args| {
                let service = weak.upgrade().ok_or("Monitter has closed.")?;
                let agent = service.mail_caller_agent(task)?;
                let result = service.mail.call_for_agent(&agent, tool, args)?;
                // A slow provider response cannot release mail after task or grant revocation.
                if service.mail_caller_agent(task)? != agent
                    || !service.mail.has_agent_access(&agent)?
                {
                    return Err("Mail access was revoked while this request was running.".into());
                }
                Ok(result)
            }))?);
        }
        let grant = broker
            .as_ref()
            .ok_or("Mail broker is unavailable.")?
            .session(task_id);
        grants.insert(task_id.to_owned(), grant.clone());
        Ok(Some(grant))
    }

    pub(crate) fn revoke_mail_grant(&self, task_id: &str) {
        // Same lock order as grant creation.
        if let Ok(mut grants) = self.mail_grants.lock() {
            if let Some(grant) = grants.remove(task_id) {
                if let Ok(broker) = self.mail_broker.lock() {
                    if let Some(broker) = broker.as_ref() {
                        broker.revoke(&grant.token);
                    }
                }
            }
        }
    }

    pub(crate) fn set_mail_account_agents(
        &self,
        revision: String,
        account_id: String,
        agent_ids: Vec<String>,
    ) -> Result<crate::mail::MailConfig, String> {
        if agent_ids.len() > 128 {
            return Err("At most 128 agents can be assigned to a mail account.".into());
        }
        let data = self.committed_data()?;
        let allowed: HashSet<&str> = data
            .snapshot
            .agents
            .iter()
            .filter(|agent| eligible(agent, &data.snapshot.hosts))
            .map(|agent| agent.id.as_str())
            .collect();
        if agent_ids.iter().any(|id| !allowed.contains(id.as_str())) {
            return Err("Mail supports local Codex, Claude and HTTP-capable ACP agents only; internal, SSH and AGY headless agents cannot be assigned.".into());
        }
        self.mail
            .set_account_agents(revision, account_id, agent_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mail_rejects_remote_unsupported_and_headless_launches() {
        assert!(supported("codex", "local", None));
        assert!(supported("claude", "local", None));
        assert!(supported("acp", "local", None));
        assert!(!supported("codex", "ssh", None));
        assert!(!supported("hermes", "local", None));
        assert!(!supported(
            "acp",
            "local",
            Some(&AcpLaunch {
                command: "/bin/monitter-agy-acp".into(),
                args: vec![]
            })
        ));
    }
    fn fixture() -> (Arc<Service>, std::path::PathBuf, String, String) {
        let root =
            std::env::temp_dir().join(format!("monitter-mail-grants-{}", crate::model::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut snapshot = crate::model::default_snapshot();
        let agent = snapshot
            .agents
            .iter_mut()
            .find(|agent| !agent.internal)
            .unwrap();
        agent.provider = "codex".into();
        let agent_id = agent.id.clone();
        let task = crate::model::task_from_agent(
            agent,
            &crate::model::CreateTaskInput {
                agent_id: agent_id.clone(),
                title: "Mail fixture".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: Some(agent.cwd.clone()),
                model_settings: None,
                sandbox: None,
            },
        );
        let task_id = task.id.clone();
        snapshot.tasks.push(task);
        std::fs::write(
            root.join("state.json"),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let service = Service::open(None, root.clone()).unwrap();
        service
            .mutate(None, |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == task_id)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        (service, root, task_id, agent_id)
    }
    #[test]
    fn monitter_mcp_auto_approval_runtime_reads_persisted_setting() {
        let (service, root, task_id, _) = fixture();
        assert!(service.runtime_extensions_for_task(&task_id).unwrap().auto_approve_monitter_mcp);
        service.mutate(None, |snapshot| {
            snapshot.settings.auto_approve_monitter_mcp = false;
            Ok(())
        }).unwrap();
        assert!(!service.runtime_extensions_for_task(&task_id).unwrap().auto_approve_monitter_mcp);
        // Saving unrelated preferences must not erase the opt-out.
        service.mutate(None, |snapshot| {
            snapshot.settings.user_name = "Fixture".into();
            Ok(())
        }).unwrap();
        assert!(!service.runtime_extensions_for_task(&task_id).unwrap().auto_approve_monitter_mcp);
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    fn rpc(grant: &SessionGrant, name: &str, args: serde_json::Value) -> String {
        use std::io::{Read, Write};
        let address = grant
            .endpoint
            .trim_start_matches("http://")
            .trim_end_matches("/mcp");
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let body = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string();
        write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-03-26\r\nContent-Length: {}\r\n\r\n{}", grant.token, body.len(), body).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
    #[test]
    fn mail_grants_are_task_scoped_rechecked_and_revoked() {
        let (service, root, task_id, agent_id) = fixture();
        assert!(service.mail_grant(&task_id).unwrap().is_none());
        let account_id = crate::model::id();
        service
            .mail
            .seed_test_account(
                crate::mail::MailAccount {
                    id: account_id.clone(),
                    provider: "gmail".into(),
                    email: "reader@example.com".into(),
                    label: "Reader".into(),
                    agent_ids: vec![agent_id],
                    connected_at: 1,
                },
                "fixture-refresh-never-transmitted".into(),
            )
            .unwrap();
        let grant = service.mail_grant(&task_id).unwrap().unwrap();
        let response = rpc(&grant, "mail_accounts", serde_json::json!({}));
        assert!(response.contains("reader@example.com"));
        assert!(!response.contains("fixture-refresh"));
        let bad = rpc(
            &grant,
            "mail_search",
            serde_json::json!({"account_id":crate::model::id()}),
        );
        assert!(bad.contains("isError"));
        let revision = service.mail.config().unwrap().revision;
        service
            .set_mail_account_agents(revision, account_id, vec![])
            .unwrap();
        let revoked = rpc(&grant, "mail_accounts", serde_json::json!({}));
        assert!(!revoked.contains("reader@example.com"));
        service.release_run(&task_id);
        assert!(rpc(&grant, "mail_accounts", serde_json::json!({})).contains("401 Unauthorized"));
        service.cleanup();
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn assignment_rejects_internal_or_unknown_agents_before_saving() {
        let (service, root, _task_id, _agent_id) = fixture();
        let revision = service.mail.config().unwrap().revision;
        let error = service
            .set_mail_account_agents(revision, crate::model::id(), vec![crate::model::id()])
            .err()
            .unwrap();
        assert!(error.contains("local Codex"));
        service.cleanup();
        let _ = std::fs::remove_dir_all(root);
    }
}
