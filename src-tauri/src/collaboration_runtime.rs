//! Own the broker and credentials for exactly the lifetime of Monitter runs.
use crate::{
    collaboration_transport::{Broker, SessionGrant},
    Service,
};
use std::{
    sync::{atomic::Ordering, Arc},
    thread,
    time::Duration,
};

impl Service {
    pub(crate) fn initialize_collaboration(self: &Arc<Self>) -> Result<(), String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err("Monitter is shutting down.".into());
        }
        {
            let mut broker = self
                .collaboration
                .lock()
                .map_err(|_| "Collaboration broker lock failed.")?;
            if broker.is_none() {
                let service = Arc::downgrade(self);
                *broker = Some(Broker::start(Arc::new(move |caller, tool, arguments| {
                    let service = service.upgrade().ok_or("Monitter has closed.")?;
                    if service.stopping.load(Ordering::Acquire) {
                        return Err("Monitter is shutting down.".into());
                    }
                    service.protocol(caller, tool, arguments)
                }))?);
            }
        }
        if !self.collaboration_started.swap(true, Ordering::AcqRel) {
            let service = Arc::downgrade(self);
            if let Err(error) = thread::Builder::new()
                .name("monitter-collaboration-queue".into())
                .spawn(move || loop {
                    let Some(current) = service.upgrade() else {
                        break;
                    };
                    if current.stopping.load(Ordering::Acquire) {
                        break;
                    }
                    current.dispatch_collaborations();
                    drop(current);
                    thread::sleep(Duration::from_millis(200));
                })
            {
                self.collaboration_started.store(false, Ordering::Release);
                return Err(format!("Could not start collaboration queue: {error}"));
            }
        }
        Ok(())
    }

    pub(crate) fn collaboration_grant(
        self: &Arc<Self>,
        task_id: &str,
    ) -> Result<Option<SessionGrant>, String> {
        let snapshot = self.snapshot()?;
        let task = snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or("Task was not found.")?;
        let agent = snapshot
            .agents
            .iter()
            .find(|agent| agent.id == task.agent_id)
            .ok_or("Agent was not found.")?;
        if !agent.collaboration_enabled {
            return Ok(None);
        }
        if task.status != "running" || self.stopping.load(Ordering::Acquire) {
            return Err("The turn ended before collaboration tools could start.".into());
        }
        self.initialize_collaboration()?;
        let mut grants = self
            .collaboration_grants
            .lock()
            .map_err(|_| "Collaboration grant lock failed.")?;
        if let Some(grant) = grants.get(task_id) {
            return Ok(Some(grant.clone()));
        }
        let broker = self
            .collaboration
            .lock()
            .map_err(|_| "Collaboration broker lock failed.")?;
        let grant = broker
            .as_ref()
            .ok_or("Collaboration broker is unavailable.")?
            .session(task_id);
        grants.insert(task_id.into(), grant.clone());
        Ok(Some(grant))
    }

    /// ACP reload may only reuse a grant that was created for this exact
    /// task's prior live turn. It cannot create a new broker capability while
    /// the durable task is completed or interrupted.
    pub(crate) fn existing_collaboration_grant(
        &self,
        task_id: &str,
    ) -> Result<Option<SessionGrant>, String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err("Monitter is shutting down.".into());
        }
        self.collaboration_grants
            .lock()
            .map_err(|_| "Collaboration grant lock failed.".to_string())
            .map(|grants| grants.get(task_id).cloned())
    }

    pub(crate) fn revoke_collaboration_grant(&self, task_id: &str) {
        let grant = self
            .collaboration_grants
            .lock()
            .ok()
            .and_then(|mut grants| grants.remove(task_id));
        if let Some(grant) = grant {
            if let Ok(broker) = self.collaboration.lock() {
                if let Some(broker) = broker.as_ref() {
                    broker.revoke(&grant.token);
                }
            }
        }
    }

    fn initialize_jev_decisions(self: &Arc<Self>) -> Result<(), String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err("Monitter is shutting down.".into());
        }
        let mut broker = self
            .jev_decisions_broker
            .lock()
            .map_err(|_| "Jev Decisions broker lock failed.")?;
        if broker.is_none() {
            let service = Arc::downgrade(self);
            *broker = Some(Broker::start_jev_decisions(Arc::new(
                move |caller, tool, arguments| {
                    let service = service.upgrade().ok_or("Monitter has closed.")?;
                    if service.stopping.load(Ordering::Acquire) {
                        return Err("Monitter is shutting down.".into());
                    }
                    if !service.jev_decisions_opted_in_running(caller)? {
                        return Err(
                            "Jev Decisions is not enabled for this running agent task.".into()
                        );
                    }
                    let (result, receipt) =
                        crate::jev_decisions::evaluate(caller, tool, arguments)?;
                    service.record_jev_decisions_receipt(&receipt);
                    Ok(result)
                },
            ))?);
        }
        Ok(())
    }

    pub(crate) fn jev_decisions_grant(
        self: &Arc<Self>,
        task_id: &str,
    ) -> Result<Option<SessionGrant>, String> {
        let enabled = self.jev_decisions_opted_in_running(task_id)?;
        if !enabled {
            // If the setting was turned off while this task was still live,
            // revoke the old capability before returning.
            self.revoke_jev_decisions_grant(task_id);
            return Ok(None);
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err("Monitter is shutting down.".into());
        }
        self.initialize_jev_decisions()?;
        let mut grants = self
            .jev_decisions_grants
            .lock()
            .map_err(|_| "Jev Decisions grant lock failed.")?;
        if let Some(grant) = grants.get(task_id) {
            return Ok(Some(grant.clone()));
        }
        let broker = self
            .jev_decisions_broker
            .lock()
            .map_err(|_| "Jev Decisions broker lock failed.")?;
        let grant = broker
            .as_ref()
            .ok_or("Jev Decisions broker is unavailable.")?
            .session(task_id);
        grants.insert(task_id.into(), grant.clone());
        Ok(Some(grant))
    }

    /// ACP session reload may only reuse a capability issued to this running
    /// task; it cannot mint a new grant after opt-in has been withdrawn.
    pub(crate) fn existing_jev_decisions_grant(
        &self,
        task_id: &str,
    ) -> Result<Option<SessionGrant>, String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err("Monitter is shutting down.".into());
        }
        if !self.jev_decisions_opted_in_running(task_id)? {
            return Ok(None);
        }
        self.jev_decisions_grants
            .lock()
            .map_err(|_| "Jev Decisions grant lock failed.".to_string())
            .map(|grants| grants.get(task_id).cloned())
    }

    pub(crate) fn revoke_jev_decisions_grant(&self, task_id: &str) {
        let grant = self
            .jev_decisions_grants
            .lock()
            .ok()
            .and_then(|mut grants| grants.remove(task_id));
        if let Some(grant) = grant {
            if let Ok(broker) = self.jev_decisions_broker.lock() {
                if let Some(broker) = broker.as_ref() {
                    broker.revoke(&grant.token);
                }
            }
        }
    }
}

#[cfg(test)]
mod jev_decisions_grant_tests {
    use super::*;
    use crate::model::{self, CreateTaskInput};
    use std::{
        fs,
        io::{Read, Write},
        net::TcpStream,
        path::PathBuf,
    };

    fn fixture(enabled: bool) -> (Arc<Service>, PathBuf, String) {
        let root =
            std::env::temp_dir().join(format!("monitter-jev-decisions-grant-{}", model::id()));
        fs::create_dir_all(&root).unwrap();
        let mut snapshot = model::default_snapshot();
        let agent = snapshot
            .agents
            .iter_mut()
            .find(|agent| !agent.internal)
            .unwrap();
        agent.provider = "codex".into();
        agent.collaboration_enabled = true;
        agent.jev_decisions_enabled = enabled;
        let task = model::task_from_agent(
            agent,
            &CreateTaskInput {
                agent_id: agent.id.clone(),
                title: "Jev grant fixture".into(),
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
        let mut task = task;
        task.status = "running".into();
        snapshot.tasks.push(task);
        fs::write(
            root.join("state.json"),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let service = Service::open(None, root.clone()).unwrap();
        service
            .mutate(None, |snapshot| {
                let task = snapshot
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == task_id)
                    .ok_or("Jev grant fixture task is missing.")?;
                task.status = "running".into();
                Ok(())
            })
            .unwrap();
        (service, root, task_id)
    }

    fn status(endpoint: &str, token: &str) -> String {
        let address = endpoint
            .trim_start_matches("http://")
            .trim_end_matches("/mcp");
        let mut stream = TcpStream::connect(address).unwrap();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-03-26\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn grants_are_opt_in_separate_and_revoked_when_the_task_ends() {
        let (disabled, disabled_root, disabled_task) = fixture(false);
        assert!(disabled
            .jev_decisions_grant(&disabled_task)
            .unwrap()
            .is_none());
        disabled.cleanup();
        let _ = fs::remove_dir_all(disabled_root);

        let (enabled, enabled_root, task_id) = fixture(true);
        let jev = enabled.jev_decisions_grant(&task_id).unwrap().unwrap();
        let collaboration = enabled.collaboration_grant(&task_id).unwrap().unwrap();
        assert_ne!(jev.endpoint, collaboration.endpoint);
        assert_ne!(jev.token, collaboration.token);
        assert!(status(&jev.endpoint, &jev.token).contains("200 OK"));
        enabled.finish(&task_id, "completed", None);
        assert!(status(&jev.endpoint, &jev.token).contains("401 Unauthorized"));
        enabled.cleanup();
        let _ = fs::remove_dir_all(enabled_root);
    }
}
