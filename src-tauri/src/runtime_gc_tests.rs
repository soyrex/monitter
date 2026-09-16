//! Deterministic service-level coverage for idle runtime retirement.

use crate::{model::CreateTaskInput, runner::RunControl, Service};
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
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("monitter-runtime-gc-{name}-{}", crate::id()));
        Self {
            service: Service::open(None, root.clone()).unwrap(),
            root,
        }
    }
    fn task(&self, title: &str, native: Option<&str>) -> crate::model::Task {
        let agent = self.service.snapshot().unwrap().agents[0].clone();
        self.service
            .create_task(CreateTaskInput {
                agent_id: agent.id,
                title: title.into(),
                native_session_id: native.map(str::to_owned),
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap()
    }
    fn idle_owner(&self, task_id: &str, resume_supported: bool) -> Arc<RunControl> {
        let control = RunControl::new(false);
        control.begin_run().unwrap();
        control.mark_resident();
        control.set_resume_supported(resume_supported);
        control.mark_idle();
        assert!(control.is_idle());
        self.service
            .runs
            .lock()
            .unwrap()
            .tasks
            .insert(task_id.into(), control.clone());
        control
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn eligible_idle_runtime_is_retired_after_timeout() {
    let f = Fixture::new("eligible");
    let task = f.task("Eligible", Some("native-eligible"));
    let owner = f.idle_owner(&task.id, true);
    // Successful retirement requires verified process ownership, not just
    // synthetic lifecycle state. Keep a real owned child waiting for EOF.
    let mut command = std::process::Command::new("/bin/cat");
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::runner::isolate_child(&mut command);
    let mut child = command.spawn().unwrap();
    let stdin = child.stdin.take();
    owner.install(child, stdin).unwrap();
    owner.capture_runtime_process_baseline().unwrap();
    let retired = f
        .service
        .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301));
    assert_eq!(retired, 1);
    assert!(!owner.is_retiring());
    assert!(!f.service.has_resident_run(&task.id));
}

#[test]
fn unsupported_resume_is_pinned() {
    let f = Fixture::new("unsupported");
    let task = f.task("Unsupported", Some("native-unsupported"));
    let owner = f.idle_owner(&task.id, false);
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}

#[test]
fn queued_delivery_blocks_collection() {
    let f = Fixture::new("queued");
    let task = f.task("Queued", Some("native-queued"));
    let owner = f.idle_owner(&task.id, true);
    f.service
        .mutate(None, |s| {
            s.queued_messages.push(crate::model::QueuedMessage {
                id: crate::id(),
                task_id: task.id.clone(),
                channel_id: None,
                text: "later".into(),
                attachment_ids: vec![],
                created_at: crate::model::now(),
                status: "queued".into(),
                error: None,
                sender_agent_id: None,
                origin: None,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}

#[test]
fn pending_approval_blocks_collection() {
    let f = Fixture::new("approval");
    let task = f.task("Approval", Some("native-approval"));
    let owner = f.idle_owner(&task.id, true);
    f.service
        .mutate(None, |s| {
            s.approval_requests.push(crate::model::ApprovalRequest {
                id: crate::id(),
                task_id: task.id.clone(),
                provider: "codex".into(),
                run_id: "run-approval".into(),
                tool: "shell".into(),
                summary: "test".into(),
                detail: "test".into(),
                risk: "medium".into(),
                status: "pending".into(),
                created_at: crate::model::now(),
                resolved_at: None,
                decision: None,
                rememberable: false,
                session_scope: None,
                rule_id: None,
                approval_scope: None,
                input: None,
                response: None,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}

#[test]
fn stale_owner_cannot_release_successor_generation() {
    let f = Fixture::new("generation");
    let task = f.task("Generation", Some("native-generation"));
    let old = f.idle_owner(&task.id, true);
    let successor = RunControl::new(false);
    successor.mark_resident();
    f.service
        .runs
        .lock()
        .unwrap()
        .tasks
        .insert(task.id.clone(), successor.clone());
    f.service.release_retired_run(&task.id, &old);
    assert!(f.service.has_resident_run(&task.id));
    assert!(!successor.is_retiring());
}

#[test]
fn just_under_five_minute_boundary_is_still_idle() {
    let f = Fixture::new("boundary");
    let task = f.task("Boundary", Some("native-boundary"));
    let owner = f.idle_owner(&task.id, true);
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(299)),
        0
    );
    assert!(owner.is_idle());
}

#[test]
fn running_task_is_pinned_even_when_owner_is_marked_idle() {
    let f = Fixture::new("running");
    let task = f.task("Quiet running", Some("native-running"));
    let owner = f.idle_owner(&task.id, true);
    f.service
        .mutate(None, |snapshot| {
            snapshot
                .tasks
                .iter_mut()
                .find(|item| item.id == task.id)
                .unwrap()
                .status = "running".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}

#[test]
fn sending_queue_entry_blocks_collection() {
    let f = Fixture::new("sending");
    let task = f.task("Sending queue", Some("native-sending"));
    let owner = f.idle_owner(&task.id, true);
    f.service
        .mutate(None, |s| {
            s.queued_messages.push(crate::model::QueuedMessage {
                id: crate::id(),
                task_id: task.id.clone(),
                channel_id: None,
                text: "in flight".into(),
                attachment_ids: vec![],
                created_at: crate::model::now(),
                status: "sending".into(),
                error: None,
                sender_agent_id: None,
                origin: None,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}

#[test]
fn accepted_fresh_send_marks_task_running_before_gc() {
    let f = Fixture::new("accepted-send");
    let task = f.task("Accepted send", Some("native-accepted"));
    let owner = f.idle_owner(&task.id, true);
    // Exercise the durable acceptance boundary without launching a provider.
    let prompt = f
        .service
        .accept_send(task.id.clone(), "fresh message".into(), vec![])
        .unwrap();
    assert!(prompt.is_some());
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}

#[test]
fn active_internal_admin_broker_request_pins_runtime() {
    let f = Fixture::new("admin-broker");
    let task_id = f.service.ensure_internal_admin_task().unwrap();
    f.service
        .mutate(None, |snapshot| {
            let task = snapshot
                .tasks
                .iter_mut()
                .find(|item| item.id == task_id)
                .unwrap();
            task.native_session_id = Some("native-admin".into());
            task.status = "idle".into();
            Ok(())
        })
        .unwrap();
    let owner = f.idle_owner(&task_id, true);
    let _receiver = f
        .service
        .admin_turn_broker
        .try_register(
            format!("req-{}", crate::id()),
            task_id.clone(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}

#[test]
fn send_waits_for_the_exact_retiring_owner_to_be_reaped() {
    let f = Fixture::new("retirement-barrier");
    let task = f.task("Retirement barrier", Some("native-barrier"));
    let owner = f.idle_owner(&task.id, true);
    assert!(owner.try_retire_idle(
        Instant::now() + Duration::from_secs(301),
        crate::runtime_gc::IDLE_RUNTIME_TIMEOUT,
    ));

    let service = Arc::clone(&f.service);
    let task_id = task.id.clone();
    let waiter = thread::spawn(move || service.available_runtime(&task_id));
    thread::sleep(Duration::from_millis(20));
    assert!(owner.is_retiring());
    f.service.release_retired_run(&task.id, &owner);
    assert!(waiter.join().unwrap().unwrap().is_none());
}

#[test]
fn cancellation_after_acceptance_prevents_late_idle_collection() {
    let f = Fixture::new("cancel-before-launch");
    let task = f.task("Cancel before launch", Some("native-cancel"));
    let owner = f.idle_owner(&task.id, true);
    let prompt = f
        .service
        .accept_send(task.id.clone(), "accepted".into(), vec![])
        .unwrap();
    assert!(prompt.is_some());
    f.service.cancel(&task.id).unwrap();
    let snapshot = f.service.snapshot().unwrap();
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .unwrap()
            .status,
        "interrupted"
    );
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}

#[test]
fn remote_host_runtime_is_pinned_for_conservative_gc() {
    let f = Fixture::new("remote-host");
    let task = f.task("Remote host", Some("native-remote"));
    let owner = f.idle_owner(&task.id, true);
    f.service
        .mutate_data(Some(task.id.clone()), |data| {
            data.task_hosts.get_mut(&task.id).unwrap().kind = "ssh".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        f.service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301)),
        0
    );
    assert!(!owner.is_retiring());
}
