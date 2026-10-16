//! Owner-local restart fence and durable, single-consumption recovery journal.
use crate::{Service, ServiceData};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestartMode {
    Wait,
    Force,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestartAgent {
    pub task_id: String,
    pub title: String,
    pub agent_id: String,
    pub agent_name: String,
}
#[derive(Clone, Debug)]
pub struct RestartStatus {
    pub mode: RestartMode,
    pub ready: bool,
    pub active_agents: Vec<RestartAgent>,
}
#[derive(Serialize, Deserialize)]
struct Intent {
    receipt: String,
    tasks: Vec<RestartAgent>,
}

pub(crate) fn admits_turn(before: &ServiceData, after: &ServiceData) -> bool {
    after
        .accepted_turns
        .iter()
        .any(|(id, turn)| before.accepted_turns.get(id) != Some(turn))
        || after.snapshot.tasks.iter().any(|task| {
            task.status == "running"
                && !before
                    .snapshot
                    .tasks
                    .iter()
                    .any(|old| old.id == task.id && old.status == "running")
        })
        || after.snapshot.queued_messages.iter().any(|message| {
            message.status == "sending"
                && !before
                    .snapshot
                    .queued_messages
                    .iter()
                    .any(|old| old.id == message.id && old.status == "sending")
        })
}
impl Service {
    fn restart_agents_locked(&self) -> Result<Vec<RestartAgent>, String> {
        let data = self.data.lock().map_err(|_| "State lock failed.")?;
        let runs = self.runs.lock().map_err(|_| "Run registry lock failed.")?;
        Ok(data
            .snapshot
            .tasks
            .iter()
            .filter(|task| {
                task.status == "running"
                    || data.accepted_turns.contains_key(&task.id)
                    || data
                        .snapshot
                        .queued_messages
                        .iter()
                        .any(|message| message.task_id == task.id && message.status == "sending")
                    || runs
                        .tasks
                        .get(&task.id)
                        .is_some_and(|control| control.is_active())
            })
            .map(|task| RestartAgent {
                task_id: task.id.clone(),
                title: task.title.clone(),
                agent_id: task.agent_id.clone(),
                agent_name: data
                    .snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.id == task.agent_id)
                    .map(|agent| agent.name.clone())
                    .unwrap_or_else(|| "Agent".into()),
            })
            .collect())
    }
    pub(crate) fn restart_active_agents(&self) -> Result<Vec<RestartAgent>, String> {
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer lock failed.")?;
        self.restart_agents_locked()
    }
    pub(crate) fn begin_restart(&self, mode: RestartMode) -> Result<RestartStatus, String> {
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer lock failed.")?;
        let mut pending = self
            .restart_mode
            .lock()
            .map_err(|_| "Restart lock failed.")?;
        let agents = self.restart_agents_locked()?;
        if mode == RestartMode::Force {
            let intent = Intent {
                receipt: crate::model::id(),
                tasks: agents.clone(),
            };
            validate_intent(&intent)?;
            let bytes = serde_json::to_vec(&intent).map_err(|e| e.to_string())?;
            if bytes.len() > 1_048_576 {
                return Err("Restart intent exceeds its size limit.".into());
            }
            let temp = self.restart_intent_path.with_extension("tmp");
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            let mut file = options
                .open(&temp)
                .map_err(|e| format!("Cannot save restart intent: {e}"))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            fs::rename(&temp, &self.restart_intent_path).map_err(|e| e.to_string())?;
            sync_parent(&self.restart_intent_path)?;
        }
        *pending = Some(mode);
        Ok(RestartStatus {
            mode,
            ready: mode == RestartMode::Force || agents.is_empty(),
            active_agents: agents,
        })
    }
    pub(crate) fn poll_restart(&self) -> Result<Option<RestartStatus>, String> {
        let _writer = self
            .state_writes
            .lock()
            .map_err(|_| "State writer lock failed.")?;
        let mode = *self
            .restart_mode
            .lock()
            .map_err(|_| "Restart lock failed.")?;
        mode.map(|mode| {
            self.restart_agents_locked().map(|agents| RestartStatus {
                mode,
                ready: mode == RestartMode::Force || agents.is_empty(),
                active_agents: agents,
            })
        })
        .transpose()
    }
    pub(crate) fn cancel_restart(self: &Arc<Self>) -> Result<(), String> {
        {
            let _writer = self
                .state_writes
                .lock()
                .map_err(|_| "State writer lock failed.")?;
            if self.restart_intent_path.exists() {
                fs::remove_file(&self.restart_intent_path).map_err(|e| e.to_string())?;
                sync_parent(&self.restart_intent_path)?;
            }
            *self
                .restart_mode
                .lock()
                .map_err(|_| "Restart lock failed.")? = None;
            self.replacement_preparing
                .store(false, std::sync::atomic::Ordering::Release);
        }
        self.dispatch_startup_queues();
        Ok(())
    }
    /// Owner-only terminal preparation. Never return success while an exact
    /// provider reader still owns its registered process/startup reservation.
    pub(crate) fn prepare_replacement(&self) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        let controls = {
            let _writer = self
                .state_writes
                .lock()
                .map_err(|_| "State writer lock failed.")?;
            let mode = self
                .restart_mode
                .lock()
                .map_err(|_| "Restart lock failed.")?
                .ok_or("No server restart is pending.")?;
            if mode == RestartMode::Wait && !self.restart_agents_locked()?.is_empty() {
                return Err("Active agents have not finished yet.".into());
            }
            self.replacement_preparing.store(true, Ordering::Release);
            let controls = self
                .runs
                .lock()
                .map_err(|_| "Run registry lock failed.")?
                .tasks
                .iter()
                .map(|(id, control)| (id.clone(), control.clone()))
                .collect::<Vec<_>>();
            for (_, control) in &controls {
                control.reserve_cancellation();
            }
            controls
        };
        // ACP readers flush mandatory permission cancellations and then call
        // terminate_owned before pointer-checked release. Startup owners also
        // remain registered until their late child install has been reaped.
        for (_, control) in &controls {
            control.cancel();
        }
        let deadline = std::time::Instant::now() + crate::runtime_gc::RUNTIME_RELEASE_TIMEOUT;
        loop {
            let remaining = {
                let _writer = self
                    .state_writes
                    .lock()
                    .map_err(|_| "State writer lock failed.")?;
                if !self.replacement_preparing.load(Ordering::Acquire) {
                    return Err("Server replacement preparation was cancelled.".into());
                }
                // Admission remains fenced under this same writer lock. No
                // new or late owner may escape the final readiness check.
                self.runs
                    .lock()
                    .map_err(|_| "Run registry lock failed.")?
                    .tasks
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
            };
            if remaining.is_empty() {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                self.replacement_preparing.store(false, Ordering::Release);
                return Err(format!("Server replacement is blocked: {} owned agent transport(s) have not finished stopping. The restart journal is preserved.", remaining.len()));
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
    pub(crate) fn recover_restart(self: &Arc<Self>) -> Result<(), String> {
        if !self.restart_intent_path.exists() {
            return Ok(());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let file = options
            .open(&self.restart_intent_path)
            .map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Restart intent must be a regular file.".into());
        }
        let mut bytes = Vec::new();
        file.take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1_048_576 {
            return Err("Restart intent exceeds its size limit.".into());
        }
        let intent: Intent =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid restart intent: {e}"))?;
        validate_intent(&intent)?;
        // Durable receipt precedes dispatch. A second startup cannot replay the
        // same turn or enter a restart loop, even if replacement crashes.
        let receipt = self
            .restart_intent_path
            .with_file_name(format!("server-restart-{}.consumed.json", intent.receipt));
        fs::rename(&self.restart_intent_path, receipt).map_err(|e| e.to_string())?;
        sync_parent(&self.restart_intent_path)?;
        for agent in intent.tasks {
            if let Err(error) = self.resume(agent.task_id.clone()) {
                self.mutate(Some(agent.task_id.clone()), |snapshot| {
                    let Some(task) = snapshot
                        .tasks
                        .iter_mut()
                        .find(|task| task.id == agent.task_id)
                    else {
                        return Ok(());
                    };
                    task.status = "error".into();
                    task.updated_at = crate::model::now();
                    snapshot.messages.push(crate::model::Message {
                        id: crate::model::id(),
                        task_id: agent.task_id.clone(),
                        role: "system".into(),
                        text: format!("Server restart could not resume this task: {error}"),
                        created_at: crate::model::now(),
                        attachments: vec![],
                        sender_agent_id: None,
                        collaboration_id: None,
                        stream_status: None,
                        phase: None,
                        response_metadata: None,
                    });
                    Ok(())
                })?;
            }
        }
        Ok(())
    }
}
fn validate_intent(intent: &Intent) -> Result<(), String> {
    if uuid::Uuid::parse_str(&intent.receipt).is_err() || intent.tasks.len() > 4096 {
        return Err("Invalid restart receipt or task count.".into());
    }
    let mut ids = std::collections::HashSet::new();
    for task in &intent.tasks {
        if uuid::Uuid::parse_str(&task.task_id).is_err()
            || uuid::Uuid::parse_str(&task.agent_id).is_err()
            || !ids.insert(&task.task_id)
        {
            return Err("Invalid or duplicate restart task identity.".into());
        }
    }
    Ok(())
}
fn sync_parent(path: &std::path::Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Restart intent has no parent directory.")?;
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("Cannot sync restart journal directory: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (std::path::PathBuf, Arc<Service>, String) {
        let dir = std::env::temp_dir().join(format!("monitter-restart-{}", crate::model::id()));
        let service = Service::open(None, dir.clone()).unwrap();
        let agent = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(crate::CreateTaskInput {
                agent_id: agent,
                title: "Restart fixture".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        (dir, service, task.id)
    }
    #[test]
    fn wait_fences_admission_and_reservation_until_cancelled() {
        let (dir, service, task) = fixture();
        assert!(service.begin_restart(RestartMode::Wait).unwrap().ready);
        assert!(service
            .mutate(Some(task.clone()), |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .is_err());
        assert!(service.reserve_run(&task).is_err());
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .tasks
                .iter()
                .find(|item| item.id == task)
                .unwrap()
                .status,
            "idle"
        );
        service.cancel_restart().unwrap();
        assert!(service.poll_restart().unwrap().is_none());
        drop(service);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn force_records_exact_ids_and_missing_session_error_survives_replay() {
        let (dir, service, task) = fixture();
        service
            .mutate(Some(task.clone()), |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        let status = service.begin_restart(RestartMode::Force).unwrap();
        assert!(status.ready);
        assert_eq!(status.active_agents.len(), 1);
        let intent: Intent =
            serde_json::from_slice(&fs::read(&service.restart_intent_path).unwrap()).unwrap();
        assert_eq!(intent.tasks[0].task_id, task);
        drop(service);
        let service = Service::open(None, dir.clone()).unwrap();
        service.recover_restart().unwrap();
        let snapshot = service.snapshot().unwrap();
        assert_eq!(
            snapshot
                .tasks
                .iter()
                .find(|item| item.id == task)
                .unwrap()
                .status,
            "error"
        );
        assert!(snapshot
            .messages
            .iter()
            .any(|message| message.task_id == task && message.text.contains("no native session")));
        let count = snapshot.messages.len();
        service.recover_restart().unwrap();
        assert_eq!(service.snapshot().unwrap().messages.len(), count);
        drop(service);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn wait_becomes_ready_after_running_fixture_finishes_and_cancel_keeps_state() {
        let (dir, service, task) = fixture();
        service
            .mutate(Some(task.clone()), |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        assert!(!service.begin_restart(RestartMode::Wait).unwrap().ready);
        service
            .mutate(Some(task.clone()), |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task)
                    .unwrap()
                    .status = "idle".into();
                Ok(())
            })
            .unwrap();
        assert!(service.poll_restart().unwrap().unwrap().ready);
        service.cancel_restart().unwrap();
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .tasks
                .iter()
                .find(|item| item.id == task)
                .unwrap()
                .status,
            "idle"
        );
        drop(service);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn invalid_journal_is_rejected_without_dispatch_or_orphan_messages() {
        let (dir, service, task) = fixture();
        let snapshot = service.snapshot().unwrap();
        let invalid = Intent {
            receipt: "../invalid".into(),
            tasks: vec![RestartAgent {
                task_id: task,
                title: "Invalid".into(),
                agent_id: snapshot.agents[0].id.clone(),
                agent_name: "Agent".into(),
            }],
        };
        fs::write(
            &service.restart_intent_path,
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert!(service.recover_restart().is_err());
        assert!(service.restart_intent_path.exists());
        assert_eq!(service.snapshot().unwrap(), snapshot);
        drop(service);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn replacement_requires_selected_restart_and_wait_drain() {
        let (dir, service, task) = fixture();
        assert!(service.prepare_replacement().is_err());
        service
            .mutate(Some(task.clone()), |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        service.begin_restart(RestartMode::Wait).unwrap();
        assert!(service.prepare_replacement().is_err());
        service
            .mutate(Some(task.clone()), |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task)
                    .unwrap()
                    .status = "idle".into();
                Ok(())
            })
            .unwrap();
        service.prepare_replacement().unwrap();
        assert!(service
            .replacement_preparing
            .load(std::sync::atomic::Ordering::Acquire));
        service.cancel_restart().unwrap();
        assert!(!service
            .replacement_preparing
            .load(std::sync::atomic::Ordering::Acquire));
        drop(service);
        let _ = fs::remove_dir_all(dir);
    }
}
