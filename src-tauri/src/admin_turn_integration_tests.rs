//! Integration tests for the resident Monitter Admin lane.
//!
//! These tests guard the lane's contract:
//! - The lazy internal task exists exactly once across repeated or concurrent
//!   calls.
//! - The broker is the single-writer correlation point: only the registered
//!   task id is accepted; late output from a different task id never leaks
//!   into the broker buffer.
//! - Empty/error turns never mutate the durable snapshot.
//! - Streamed assistant text is captured into the broker and never lands in
//!   `Snapshot.messages`, the compact LAN projection, or any `RunEvent` page.

use crate::{
    admin_turn_broker::{AdminTurnBroker, AdminTurnReply},
    lan_sync,
    model::*,
    runner::Parsed,
    Service,
};
use std::{
    fs,
    path::PathBuf,
    sync::{mpsc, Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

struct ServiceFixture {
    service: Arc<Service>,
    root: PathBuf,
}

impl Drop for ServiceFixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(name: &str) -> ServiceFixture {
    let root = std::env::temp_dir().join(format!("monitter-admin-lane-{name}-{}", crate::id()));
    let service = Service::open(None, root.clone()).unwrap();
    ServiceFixture { service, root }
}

#[test]
fn ensure_internal_admin_task_returns_a_single_task_across_repeated_calls() {
    let fixture = fixture("lazy-single");
    let first = fixture
        .service
        .ensure_internal_admin_task()
        .expect("first lazy creation must succeed");
    let second = fixture
        .service
        .ensure_internal_admin_task()
        .expect("second lazy creation must succeed");
    let third = fixture
        .service
        .ensure_internal_admin_task()
        .expect("third lazy creation must succeed");
    assert_eq!(first, second);
    assert_eq!(second, third);
    let snapshot = fixture.service.snapshot().unwrap();
    let internal_tasks = snapshot
        .tasks
        .iter()
        .filter(|task| task.agent_id == fixture.service.internal_admin().unwrap().id)
        .collect::<Vec<_>>();
    assert_eq!(
        internal_tasks.len(),
        1,
        "lazy creation must yield exactly one internal task"
    );
    assert_eq!(internal_tasks[0].id, first);
    assert_eq!(internal_tasks[0].title, Service::INTERNAL_ADMIN_TASK_TITLE);
    assert!(!internal_tasks[0].archived);
    // The task must carry the admin's saved provider/model/host/cwd/sandbox.
    let admin = fixture.service.internal_admin().unwrap();
    assert_eq!(internal_tasks[0].provider, admin.provider);
    assert_eq!(internal_tasks[0].host_id, admin.host_id);
    assert_eq!(internal_tasks[0].model, admin.model);
    assert_eq!(internal_tasks[0].sandbox, admin.sandbox);
    // The internal task must not be a chat recipient — the public create
    // path still rejects the admin.
    let err = fixture
        .service
        .create_task(CreateTaskInput {
            agent_id: admin.id.clone(),
            title: "should fail".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .expect_err("public create_task must refuse the admin");
    assert!(err.contains("Monitter Admin agent cannot be selected as a chat recipient"));
}

#[test]
fn ensure_internal_admin_task_is_safe_under_concurrent_calls() {
    let fixture = fixture("lazy-concurrent");
    let service = Arc::clone(&fixture.service);
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let service = Arc::clone(&service);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            service
                .ensure_internal_admin_task()
                .expect("concurrent lazy creation must succeed")
        }));
    }
    let mut ids = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        1,
        "all concurrent calls must observe the same task id"
    );
    let snapshot = fixture.service.snapshot().unwrap();
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .filter(|task| task.agent_id == fixture.service.internal_admin().unwrap().id)
            .count(),
        1
    );
}

#[test]
fn is_internal_admin_task_only_matches_the_lazy_admin_task() {
    let fixture = fixture("is-internal");
    let admin_id = fixture.service.internal_admin().unwrap().id;
    let internal = fixture.service.ensure_internal_admin_task().unwrap();
    // A user task for the same admin agent does not exist (create_task rejects
    // the admin), but a different agent's task must also fail the gate.
    let user_agent = fixture
        .service
        .snapshot()
        .unwrap()
        .agents
        .into_iter()
        .find(|agent| agent.id != admin_id)
        .expect("default agent must exist");
    let user_task = fixture
        .service
        .create_task(CreateTaskInput {
            agent_id: user_agent.id.clone(),
            title: "user task".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    assert!(fixture.service.is_internal_admin_task(&internal));
    assert!(!fixture.service.is_internal_admin_task(&user_task.id));
    assert!(!fixture.service.is_internal_admin_task("does-not-exist"));
}

#[test]
fn streamed_assistant_text_never_appears_in_snapshot_messages_or_events() {
    let fixture = fixture("runtime-only");
    let task_id = fixture.service.ensure_internal_admin_task().unwrap();
    let owner: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let _receiver = fixture
        .service
        .admin_turn_broker
        .try_register(
            format!("req-{}", crate::id()),
            task_id.clone(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    // Stream three deltas through the broker, then complete.
    assert!(fixture
        .service
        .admin_turn_broker
        .capture_assistant_text(&task_id, "hello "));
    assert!(fixture
        .service
        .admin_turn_broker
        .capture_assistant_text(&task_id, "world"));
    assert!(fixture
        .service
        .admin_turn_broker
        .capture_assistant_text(&task_id, "!"));
    assert!(fixture.service.admin_turn_broker.complete(&task_id));
    let snapshot = fixture.service.snapshot().unwrap();
    assert!(
        snapshot.messages.iter().all(|message| {
            message.task_id != task_id
                && !message.text.contains("hello")
                && !message.text.contains("world")
        }),
        "internal admin text must never enter Snapshot.messages: {:#?}",
        snapshot.messages
    );
    assert!(
        snapshot.events.iter().all(|event| event.task_id != task_id),
        "internal admin text must never enter Snapshot.events"
    );
    // Compact LAN projection must also omit it.
    let compact = lan_sync::compact_snapshot(&snapshot);
    assert!(compact.events.iter().all(|event| event.task_id != task_id));
    assert!(compact
        .messages
        .iter()
        .all(|message| message.task_id != task_id));
    // Event pages must be empty for the internal task.
    let page = lan_sync::task_events(&snapshot, &task_id, None, Some(100));
    assert!(
        page.events.is_empty(),
        "internal admin task must have no paged events"
    );
}

#[test]
fn empty_prompt_does_not_create_an_internal_task_or_register_a_broker_request() {
    let fixture = fixture("empty-prompt");
    let broker = AdminTurnBroker::new();
    let owner: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let receiver = broker
        .try_register(
            "req-empty".into(),
            "task-empty".into(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    // Capture with an empty delta is a no-op but is not rejected: the
    // broker does not silently drop text on the canonical path.
    assert!(broker.capture_assistant_text("task-empty", ""));
    // Cancelling silently clears the entry without invoking the callback.
    assert!(broker.cancel_silently("task-empty"));
    assert_eq!(
        receiver.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    let snapshot = fixture.service.snapshot().unwrap();
    // The fixture never called ensure_internal_admin_task, so the internal
    // task does not exist and no events were recorded.
    assert!(snapshot
        .tasks
        .iter()
        .all(|task| task.title != Service::INTERNAL_ADMIN_TASK_TITLE));
    assert!(snapshot.events.is_empty());
}

#[test]
fn late_output_from_a_completed_turn_is_dropped_silently() {
    let fixture = fixture("late-output");
    let task_id = fixture.service.ensure_internal_admin_task().unwrap();
    let owner: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let _receiver = fixture
        .service
        .admin_turn_broker
        .try_register(
            format!("req-{}", crate::id()),
            task_id.clone(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert!(fixture
        .service
        .admin_turn_broker
        .capture_assistant_text(&task_id, "first"));
    assert!(fixture.service.admin_turn_broker.complete(&task_id));
    // Late deltas after completion must be ignored entirely.
    assert!(!fixture
        .service
        .admin_turn_broker
        .capture_assistant_text(&task_id, "late"));
    // An unknown task id is also dropped silently.
    assert!(!fixture
        .service
        .admin_turn_broker
        .capture_assistant_text("ghost", "stale"));
    // No event/messages should have been recorded.
    let snapshot = fixture.service.snapshot().unwrap();
    assert!(snapshot.events.iter().all(|event| event.task_id != task_id));
    assert!(snapshot
        .messages
        .iter()
        .all(|message| message.task_id != task_id));
}

#[test]
fn cancelled_request_never_persists_its_buffer() {
    let fixture = fixture("cancel-silent");
    let task_id = fixture.service.ensure_internal_admin_task().unwrap();
    let owner: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let _receiver = fixture
        .service
        .admin_turn_broker
        .try_register(
            format!("req-{}", crate::id()),
            task_id.clone(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    // Stream text then cancel silently.
    assert!(fixture
        .service
        .admin_turn_broker
        .capture_assistant_text(&task_id, "leak-attempt"));
    assert!(fixture.service.admin_turn_broker.cancel_silently(&task_id));
    let snapshot = fixture.service.snapshot().unwrap();
    assert!(snapshot
        .messages
        .iter()
        .all(|message| { !message.text.contains("leak-attempt") && message.task_id != task_id }));
    assert!(snapshot.events.iter().all(|event| event.task_id != task_id));
}

#[test]
fn app_server_message_redirection_skips_snapshot_messages() {
    // Simulate the Codex app-server ingestion path: `app_server_message`
    // routes streamed deltas through the broker when the task is the
    // internal admin's, leaving the snapshot untouched.
    let fixture = fixture("redirection");
    let task_id = fixture.service.ensure_internal_admin_task().unwrap();
    let owner: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let _receiver = fixture
        .service
        .admin_turn_broker
        .try_register(
            format!("req-{}", crate::id()),
            task_id.clone(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    fixture
        .service
        .admin_turn_broker
        .capture_assistant_text(&task_id, "streamed ");
    fixture
        .service
        .admin_turn_broker
        .capture_assistant_text(&task_id, "reply");
    assert!(fixture.service.admin_turn_broker.complete(&task_id));
    let snapshot = fixture.service.snapshot().unwrap();
    assert!(snapshot.messages.iter().all(|message| {
        message.task_id != task_id
            && !message.text.contains("streamed")
            && !message.text.contains("reply")
    }));
    assert!(snapshot.events.iter().all(|event| event.task_id != task_id));
}

#[test]
fn watchdog_terminates_owner_and_drops_buffer_after_timeout() {
    let fixture = fixture("watchdog-timeout");
    let task_id = fixture.service.ensure_internal_admin_task().unwrap();
    let owner: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let broker = fixture.service.admin_turn_broker.clone();
    let request_id = format!("req-{}", crate::id());
    let receiver = broker
        .try_register(
            request_id.clone(),
            task_id.clone(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_millis(50),
        )
        .unwrap();
    let owner_for_timeout = Arc::clone(&owner);
    let broker_weak = broker.downgrade();
    let owner_weak = Arc::downgrade(&owner);
    crate::admin_turn_broker::spawn_admin_turn_watchdog(
        broker_weak,
        task_id.clone(),
        owner_weak,
        move || {
            owner_for_timeout.terminate_owned();
        },
    );
    // Streaming during the deadline window must be dropped on timeout.
    assert!(broker.capture_assistant_text(&task_id, "leak-before-timeout"));
    let reply = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("watchdog must deliver a reply");
    assert!(matches!(reply, AdminTurnReply::Timeout));
    // Timeout delivery precedes the teardown callback. Wait for that separate
    // observable action rather than racing the receiving thread against it.
    let cancellation_deadline = Instant::now() + Duration::from_secs(2);
    while !owner.is_cancelled() && Instant::now() < cancellation_deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(owner.is_cancelled(), "owner must be cancelled by watchdog");
    // After the timeout, the broker rejects further captures from any task.
    assert!(!broker.capture_assistant_text(&task_id, "late"));
}

#[test]
fn apply_event_redirection_skips_snapshot_messages_for_admin_task() {
    // Simulate the Claude/generic ingestion path: `apply_event` with assistant
    // text routes the reply through the broker when the task is the internal
    // admin's, leaving the snapshot untouched.
    let fixture = fixture("apply-event-redirect");
    let task_id = fixture.service.ensure_internal_admin_task().unwrap();
    let parsed = Parsed {
        native_session_id: None,
        assistant: Some("would-have-been-persisted".into()),
        event: Some((
            "output".into(),
            "Assistant response".into(),
            "would-have-been-persisted".into(),
        )),
        failed: false,
    };
    fixture
        .service
        .apply_event(&task_id, parsed)
        .expect("apply_event must accept the internal admin task without error");
    let snapshot = fixture.service.snapshot().unwrap();
    assert!(snapshot
        .messages
        .iter()
        .all(|message| message.task_id != task_id
            && !message.text.contains("would-have-been-persisted")));
    assert!(snapshot.events.iter().all(|event| event.task_id != task_id));
}

#[test]
fn app_server_event_redirection_skips_snapshot_events_for_admin_task() {
    let fixture = fixture("app-server-event-redirect");
    let task_id = fixture.service.ensure_internal_admin_task().unwrap();
    let control: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    control.mark_resident();
    control.set_app_server_thread("native-session-1".into());
    control.set_app_server_turn("turn-1".into());
    fixture
        .service
        .mutate_data(None, |data| {
            data.snapshot
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id)
                .unwrap()
                .status = "running".into();
            Ok(())
        })
        .unwrap();
    fixture
        .service
        .runs
        .lock()
        .unwrap()
        .tasks
        .insert(task_id.clone(), Arc::clone(&control));
    let parsed = Parsed {
        native_session_id: Some("native-session-1".into()),
        assistant: Some("would-have-been-persisted".into()),
        event: Some((
            "tool".into(),
            "Tool activity".into(),
            "would-have-leaked".into(),
        )),
        failed: false,
    };
    fixture
        .service
        .app_server_event(&task_id, &control, Some("turn-1"), parsed)
        .expect("app_server_event must accept the internal admin task without error");
    let snapshot = fixture.service.snapshot().unwrap();
    assert!(snapshot.events.iter().all(|event| event.task_id != task_id));
    assert!(snapshot
        .messages
        .iter()
        .all(|message| message.task_id != task_id));
    // The invisible admin still retains its native session for a later cold
    // wake; only transcript/activity projection is suppressed.
    let task = snapshot
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .expect("internal task must still exist");
    assert_eq!(task.native_session_id.as_deref(), Some("native-session-1"));
}

#[test]
fn broker_rejects_concurrent_registration_with_a_readable_error() {
    let broker = AdminTurnBroker::new();
    let owner_a: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let owner_b: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let receiver_a = broker
        .try_register(
            "req-a".into(),
            "task-a".into(),
            Arc::downgrade(&owner_a),
            Instant::now() + Duration::from_secs(5),
        )
        .expect("first registration must succeed");
    let second = broker.try_register(
        "req-b".into(),
        "task-b".into(),
        Arc::downgrade(&owner_b),
        Instant::now() + Duration::from_secs(5),
    );
    let error = second.expect_err("second registration must reject");
    assert!(
        error.contains("Monitter Admin is busy with another interface request"),
        "unexpected error: {error}"
    );
    drop(receiver_a);
}

#[test]
fn broker_captures_only_for_the_owned_task_id() {
    let broker = AdminTurnBroker::new();
    let owner: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let receiver = broker
        .try_register(
            format!("req-{}", crate::id()),
            "owned-task".into(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert!(broker.capture_assistant_text("owned-task", "alpha"));
    // Foreign task id must not append.
    assert!(!broker.capture_assistant_text("foreign-task", "beta"));
    assert!(broker.complete("owned-task"));
    // Late complete on a different task id is a no-op.
    assert!(!broker.complete("foreign-task"));
    let reply = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(reply, AdminTurnReply::Text("alpha".into()));
}

#[test]
fn broker_ignores_late_completion_when_a_request_was_already_timed_out() {
    let broker = AdminTurnBroker::new();
    let owner: Arc<crate::runner::RunControl> = crate::runner::RunControl::new(false);
    let receiver = broker
        .try_register(
            "req-timed".into(),
            "task-timed".into(),
            Arc::downgrade(&owner),
            Instant::now() + Duration::from_millis(50),
        )
        .unwrap();
    let broker_weak = broker.downgrade();
    let owner_weak = Arc::downgrade(&owner);
    let owner_for_timeout = Arc::clone(&owner);
    crate::admin_turn_broker::spawn_admin_turn_watchdog(
        broker_weak,
        "task-timed".into(),
        owner_weak,
        move || {
            owner_for_timeout.terminate_owned();
        },
    );
    // Capture a tiny amount of text before the timeout fires.
    assert!(broker.capture_assistant_text("task-timed", "stale"));
    let reply = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("watchdog must deliver a reply");
    assert!(matches!(reply, AdminTurnReply::Timeout));
    // Give the watchdog a chance to finish `terminate_owned` on its own
    // thread before we assert the owner is cancelled. Without this small
    // grace the watchdog may still be running `cancel()` when the
    // assertion fires, racing the test thread on the SeqCst flag.
    thread::sleep(Duration::from_millis(50));
    // The transport must already be cancelled; a late completion must be a
    // no-op that does not invent a new reply.
    assert!(owner.is_cancelled());
    assert!(!broker.complete("task-timed"));
}
