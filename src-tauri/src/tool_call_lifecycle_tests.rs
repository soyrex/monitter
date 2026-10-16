//! Exercise the durable ingestion path without starting a provider process.
use super::*;
use serde_json::json;

struct Fixture {
    service: Arc<Service>,
    root: std::path::PathBuf,
    task: String,
    control: Arc<runner::RunControl>,
    preserve_root: bool,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("monitter-tool-lifecycle-{}", id()));
        let service = Service::open(None, root.clone()).unwrap();
        let agent = service.snapshot().unwrap().agents[0].clone();
        let task = service.create_task(CreateTaskInput {
            agent_id: agent.id, title: "Lifecycle fixture".into(), native_session_id: None,
            parent_task_id: None, channel_id: None, project_id: None, cwd: None,
            model_settings: None, sandbox: None,
        }).unwrap();
        service.mutate(None, |snapshot| {
            snapshot.tasks.iter_mut().find(|t| t.id == task.id).unwrap().provider = "codex".into();
            Ok(())
        }).unwrap();
        let control = runner::RunControl::new(false);
        control.begin_run().unwrap();
        service.runs.lock().unwrap().tasks.insert(task.id.clone(), control.clone());
        Self { service, root, task: task.id, control, preserve_root: false }
    }
    fn emit(&self, item: &str, status: &str) -> String {
        let detail = json!({"type":"mcpToolCall","id":item,"server":"monitter",
            "tool":"build_request","status":status,"arguments":{"requestId":"same-request"},
            "result":{"content":[{"type":"text","text":"full result provenance"}]}}).to_string();
        self.service.apply_event(&self.task, Parsed {
            native_session_id: None, assistant: None, failed: false,
            event: Some(("tool".into(), "build_request".into(), detail.clone())),
        }).unwrap();
        detail
    }
    fn tools(&self) -> Vec<Arc<RunEvent>> {
        self.service.committed_data().unwrap().snapshot.events.iter()
            .filter(|event| event.task_id == self.task && event.kind == "tool").cloned().collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.runs.lock().unwrap().tasks.remove(&self.task);
        self.service.cleanup();
        if !self.preserve_root {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn tool_call_lifecycle_updates_identity_and_durable_detail_without_reopening_running_status() {
    let mut f = Fixture::new();
    f.emit("item-a", "inProgress");
    let initial = f.tools()[0].clone();
    let completed = f.emit("item-a", "completed");
    f.emit("item-a", "inProgress");
    let rows = f.tools();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, initial.id);
    assert_eq!(rows[0].created_at, initial.created_at);
    assert_eq!(rows[0].detail.as_ref(), completed);
    assert_eq!(f.service.store.task_event(&f.task, &initial.id).unwrap().detail.as_ref(), completed);
    // Release the service's profile owner lock before reopening the same data.
    // Fixture teardown still stops its isolated runtime, but keeps SQLite here.
    let root = f.root.clone();
    let task = f.task.clone();
    f.preserve_root = true;
    drop(f);
    let reopened = Service::open(None, root.clone()).unwrap();
    assert_eq!(reopened.store.task_event(&task, &initial.id).unwrap().detail.as_ref(), completed);
    reopened.cleanup();
    drop(reopened);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tool_call_lifecycle_distinguishes_provider_items_and_later_sends() {
    let f = Fixture::new();
    f.emit("item-a", "completed");
    f.emit("item-b", "completed");
    let original = f.tools();
    assert_eq!(original.len(), 2);
    assert_ne!(original[0].id, original[1].id);
    f.control.begin_run().unwrap();
    f.emit("item-a", "inProgress");
    let rows = f.tools();
    assert_eq!(rows.len(), 3);
    assert_ne!(rows[0].id, rows[2].id);
}

#[test]
fn tool_call_lifecycle_updates_cold_history_with_full_result() {
    let f = Fixture::new();
    f.emit("item-a", "inProgress");
    let initial = f.tools()[0].clone();
    for index in 0..(resident_history::HOT_EVENTS + 1) {
        f.service.apply_event(&f.task, Parsed {
            native_session_id: None, assistant: None, failed: false,
            event: Some(("log".into(), "intervening".into(), format!("entry {index}"))),
        }).unwrap();
    }
    assert!(f.service.committed_data().unwrap().snapshot.events.payload_is_cold(&initial.id));
    let completed = f.emit("item-a", "completed");
    assert_eq!(f.tools().len(), 1);
    assert_eq!(f.service.store.task_event(&f.task, &initial.id).unwrap().detail.as_ref(), completed);
    f.emit("item-a", "inProgress");
    assert_eq!(f.service.store.task_event(&f.task, &initial.id).unwrap().detail.as_ref(), completed);
}

#[test]
fn tool_call_lifecycle_command_and_file_updates_replace_their_own_rows() {
    let f = Fixture::new();
    for kind in ["commandExecution", "fileChange"] {
        for status in ["inProgress", "completed"] {
            let detail = json!({"type":kind,"id":"same-provider-item-id","status":status,"command":"pwd"}).to_string();
            f.service.apply_event(&f.task, Parsed {
                native_session_id: None, assistant: None, failed: false,
                event: Some(("tool".into(), kind.into(), detail)),
            }).unwrap();
        }
    }
    let rows = f.tools();
    assert_eq!(rows.len(), 2, "tool types do not collide even with a reused provider id");
    for row in rows {
        assert_eq!(serde_json::from_str::<serde_json::Value>(&row.detail).unwrap()["status"], "completed");
    }
}
