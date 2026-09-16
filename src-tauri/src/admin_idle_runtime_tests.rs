//! Lifecycle coverage for the invisible resident Monitter Admin lane.

use crate::{ApprovalDecision, Service};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    service: Arc<Service>,
    root: PathBuf,
    executable: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_file(&self.executable);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!("monitter-admin-idle-{}", crate::id()));
    let executable = std::env::temp_dir().join(format!("monitter-admin-idle-{}.mjs", crate::id()));
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/fixtures/codex-app-server/mock.mjs"),
    )
    .unwrap();
    // The internal admin deliberately has collaboration disabled. Keep the
    // mock's saved-thread/excludeTurns assertion, but do not require a
    // Monitter MCP helper that this lane must never create.
    let source = source.replace(
        "if (request.params?.excludeTurns !== true || config['mcp_servers.monitter.required'] !== true || config['mcp_servers.monitter.command'] !== 'python3') {",
        "if (request.params?.excludeTurns !== true) {",
    );
    fs::write(&executable, source).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let service = Service::open(None, root.clone()).unwrap();
    let admin = service.internal_admin().unwrap();
    service
        .mutate_data(None, |data| {
            data.snapshot
                .hosts
                .iter_mut()
                .find(|h| h.id == admin.host_id)
                .unwrap()
                .codex_path = executable.to_string_lossy().into();
            Ok(())
        })
        .unwrap();
    Fixture {
        service,
        root,
        executable,
    }
}

fn resolve(service: &Arc<Service>, task: &str) {
    for _ in 0..3 {
        let deadline = Instant::now() + Duration::from_secs(5);
        let request = loop {
            if let Some(request) = service
                .snapshot()
                .unwrap()
                .approval_requests
                .into_iter()
                .find(|r| r.task_id == task && r.status == "pending")
            {
                break request;
            }
            assert!(
                Instant::now() < deadline,
                "timed out awaiting fixture approval"
            );
            thread::sleep(Duration::from_millis(10));
        };
        if request.input.is_some() {
            service
                .resolve_input_request(
                    &request.id,
                    json!({"answers":{"confirm":{"answers":["yes"]}}}),
                )
                .unwrap();
        } else {
            service
                .resolve_approval_request(&request.id, ApprovalDecision::ApproveOnce)
                .unwrap();
        }
    }
}

fn admin_turn(f: &Fixture, prompt: &str) -> String {
    let service = Arc::clone(&f.service);
    let prompt = prompt.to_string();
    let handle = thread::spawn(move || service.send_admin_turn(prompt));
    let task = f.service.ensure_internal_admin_task().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while f.service.admin_turn_broker.active_task_id().as_deref() != Some(task.as_str()) {
        if handle.is_finished() {
            return handle.join().unwrap().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "admin request was not registered: {:?}",
            f.service.snapshot().unwrap().tasks
        );
        thread::sleep(Duration::from_millis(10));
    }
    resolve(&f.service, &task);
    handle.join().unwrap().unwrap()
}

#[test]
fn admin_reuses_then_cold_resumes_without_snapshot_leakage() {
    let f = fixture();
    let task = f.service.ensure_internal_admin_task().unwrap();
    assert_eq!(admin_turn(&f, "first"), "fixture response");
    let first = f
        .service
        .runs
        .lock()
        .unwrap()
        .tasks
        .get(&task)
        .cloned()
        .unwrap();
    assert_eq!(admin_turn(&f, "second"), "fixture response");
    let second = f
        .service
        .runs
        .lock()
        .unwrap()
        .tasks
        .get(&task)
        .cloned()
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let native = f
        .service
        .snapshot()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == task)
        .unwrap()
        .native_session_id
        .clone()
        .unwrap();
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        1
    );
    assert!(f.service.runs.lock().unwrap().tasks.get(&task).is_none());
    assert_eq!(admin_turn(&f, "third"), "fixture response");
    let third = f
        .service
        .runs
        .lock()
        .unwrap()
        .tasks
        .get(&task)
        .cloned()
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &third));
    let snapshot = f.service.snapshot().unwrap();
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .find(|t| t.id == task)
            .unwrap()
            .native_session_id
            .as_deref(),
        Some(native.as_str())
    );
    assert!(snapshot.messages.iter().all(|m| m.task_id != task));
    assert!(snapshot.events.iter().all(|e| e.task_id != task));
}

#[test]
fn concurrent_admin_send_is_rejected_before_a_second_prompt_is_dispatched() {
    let f = fixture();
    let task = f.service.ensure_internal_admin_task().unwrap();
    let service = Arc::clone(&f.service);
    let first = thread::spawn(move || service.send_admin_turn("first".into()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while f.service.admin_turn_broker.active_task_id().as_deref() != Some(task.as_str()) {
        assert!(
            Instant::now() < deadline,
            "first admin request was not registered"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let error = f.service.send_admin_turn("second".into()).unwrap_err();
    assert!(
        error.contains("busy with another interface request"),
        "{error}"
    );
    resolve(&f.service, &task);
    assert_eq!(first.join().unwrap().unwrap(), "fixture response");
}
