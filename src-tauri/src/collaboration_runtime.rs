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
}
