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
    // mock's saved-thread/excludeTurns assertion, but do not require an MCP
    // server for this lane. Locate the validation line so the fixture can
    // evolve its normal HTTP MCP checks without weakening them elsewhere.
    let validation = source
        .lines()
        .find(|line| line.trim_start().starts_with("if (request.params?.excludeTurns !== true || config['mcp_servers.monitter.required']"))
        .expect("Codex fixture resume validation");
    let source = source.replacen(
        validation,
        "    if (request.params?.excludeTurns !== true) {",
        1,
    );
    // Independent pool chats need distinct native sessions, just like real
    // providers. Keep this local to the fixture instead of changing process env.
    let source = source.replace(
        "const uniqueIds = process.env.MONITTER_FIXTURE_UNIQUE_IDS === '1';",
        "const uniqueIds = true;",
    );
    // This fixture has no configured MCP transports. Advertise the actual
    // empty inventory/config so startup ownership can be verified, rather than
    // exercising the unsupported-provider fail-closed fallback accidentally.
    let source = source.replace(
        "  if (request.method === 'initialized') return;",
        "  if (request.method === 'initialized') return;\n  if (request.method === 'mcpServerStatus/list') return response(request.id, {data: [], nextCursor: null});\n  if (request.method === 'config/read') return response(request.id, {config: {mcp_servers: {}}, origins: {}, layers: null});",
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
            .collect_idle_runtimes_at(Instant::now() + (crate::runtime_gc::IDLE_RUNTIME_TIMEOUT + Duration::from_secs(1))),
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
fn three_hidden_admin_chats_run_concurrently_and_fourth_waits_then_drains() {
    let f = fixture();
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    let mut handles = Vec::new();
    for index in 0..3 {
        let service = Arc::clone(&f.service);
        let reply = reply_tx.clone();
        handles.push(thread::spawn(move || {
            let result = service.send_admin_turn(format!("pool request {index}"));
            reply.send((index, result)).unwrap();
        }));
    }
    let admin = f.service.internal_admin().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let tasks = loop {
        let snapshot = f.service.snapshot().unwrap();
        let tasks = snapshot
            .tasks
            .iter()
            .filter(|task| task.agent_id == admin.id)
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        let all_active = tasks.len() == 3
            && tasks.iter().all(|task| {
                f.service.admin_turn_broker.is_active_task(task)
                    && snapshot
                        .approval_requests
                        .iter()
                        .any(|r| r.task_id == *task && r.status == "pending")
            });
        if all_active {
            break tasks;
        }
        assert!(
            reply_rx.try_recv().is_err(),
            "an initial request failed before all three slots ran"
        );
        assert!(
            Instant::now() < deadline,
            "three independent admin turns did not start"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let service = Arc::clone(&f.service);
    handles.push(thread::spawn(move || {
        let result = service.send_admin_turn("pool request 3".into());
        reply_tx.send((3, result)).unwrap();
    }));
    thread::sleep(Duration::from_millis(100));
    assert!(
        !handles[3].is_finished(),
        "fourth request must queue without a busy error"
    );
    assert!(reply_rx.try_recv().is_err());
    assert_eq!(
        f.service
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .filter(|task| task.agent_id == admin.id)
            .count(),
        3
    );

    // Completing one turn admits the fourth on that same hidden task; the
    // other two turns remain independently blocked on fixture approvals.
    resolve(&f.service, &tasks[0]);
    let first = reply_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_ne!(first.0, 3);
    assert_eq!(first.1.unwrap(), "fixture response");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = f.service.snapshot().unwrap();
        if f.service.admin_turn_broker.is_active_task(&tasks[0])
            && snapshot
                .approval_requests
                .iter()
                .any(|request| request.task_id == tasks[0] && request.status == "pending")
        {
            break;
        }
        assert!(
            !handles[3].is_finished(),
            "queued turn failed instead of dispatching"
        );
        assert!(
            Instant::now() < deadline,
            "fourth request did not use the released slot"
        );
        thread::sleep(Duration::from_millis(10));
    }
    resolve(&f.service, &tasks[1]);
    resolve(&f.service, &tasks[2]);
    resolve(&f.service, &tasks[0]);
    let mut completed = std::collections::HashSet::from([first.0]);
    for _ in 0..3 {
        let (index, result) = reply_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(result.unwrap(), "fixture response");
        completed.insert(index);
    }
    assert_eq!(completed.len(), 4);
    for handle in handles {
        handle.join().unwrap();
    }
    let snapshot = f.service.snapshot().unwrap();
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .filter(|task| task.agent_id == admin.id)
            .count(),
        3
    );
    assert!(snapshot
        .messages
        .iter()
        .all(|message| !tasks.contains(&message.task_id)));
    assert!(snapshot
        .queued_messages
        .iter()
        .all(|message| !tasks.contains(&message.task_id)));
    assert!(snapshot
        .events
        .iter()
        .all(|event| !event.detail.contains("pool request")));
}
