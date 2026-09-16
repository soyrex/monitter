use super::*;
use std::{
    thread,
    time::{Duration, Instant},
};

fn service_and_task() -> (Arc<Service>, Task, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("monitter-accepted-dispatch-{}", id()));
    let service = Service::open(None, root.clone()).unwrap();
    let agent = service.snapshot().unwrap().agents[0].clone();
    let task = service
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "receipt test".into(),
            native_session_id: Some("shared-native".into()),
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    (service, task, root)
}

#[test]
fn stale_receipt_failure_cannot_cancel_a_later_turn_on_the_same_owner() {
    let (service, task, root) = service_and_task();
    service
        .mutate_data(None, |data| {
            data.snapshot
                .tasks
                .iter_mut()
                .find(|value| value.id == task.id)
                .unwrap()
                .status = "running".into();
            data.accepted_turns.insert(
                task.id.clone(),
                AcceptedTurn {
                    receipt: "old".into(),
                    prompt: "old prompt".into(),
                },
            );
            Ok(())
        })
        .unwrap();
    let control = service.reserve_run(&task.id).unwrap();
    let old_run = control.current_run_id().unwrap();
    control.begin_run().unwrap();
    service.fail_accepted_with_control(&task.id, "old", &control, &old_run, "late failure".into());

    assert!(!control.is_cancelled());
    assert_eq!(
        service.data.lock().unwrap().accepted_turns[&task.id].receipt,
        "old"
    );
    service.release_run_if_current(&task.id, &control);
    service.cleanup();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn native_session_conflict_is_rejected_without_replacing_the_existing_owner() {
    let (service, task, root) = service_and_task();
    let second = service
        .create_task(CreateTaskInput {
            agent_id: service.snapshot().unwrap().agents[0].id.clone(),
            title: "conflict".into(),
            native_session_id: Some("shared-native".into()),
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    service
        .mutate_data(None, |data| {
            for id in [&task.id, &second.id] {
                data.snapshot
                    .tasks
                    .iter_mut()
                    .find(|value| &value.id == id)
                    .unwrap()
                    .status = "running".into();
            }
            data.accepted_turns.insert(
                second.id.clone(),
                AcceptedTurn {
                    receipt: "second".into(),
                    prompt: "p".into(),
                },
            );
            Ok(())
        })
        .unwrap();
    let first = service.reserve_run(&task.id).unwrap();
    service.deliver_accepted(
        second.id.clone(),
        AcceptedTurn {
            receipt: "second".into(),
            prompt: "p".into(),
        },
    );
    assert!(service
        .runs
        .lock()
        .unwrap()
        .tasks
        .get(&task.id)
        .is_some_and(|owner| Arc::ptr_eq(owner, &first)));
    assert_eq!(
        service
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .find(|value| value.id == second.id)
            .unwrap()
            .status,
        "error"
    );
    service.release_run_if_current(&task.id, &first);
    service.cleanup();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cancelled_waiting_receipt_cannot_start_after_retirement_releases() {
    let (service, task, root) = service_and_task();
    service
        .mutate_data(None, |data| {
            data.snapshot
                .tasks
                .iter_mut()
                .find(|value| value.id == task.id)
                .unwrap()
                .status = "running".into();
            data.accepted_turns.insert(
                task.id.clone(),
                AcceptedTurn {
                    receipt: "old".into(),
                    prompt: "old prompt".into(),
                },
            );
            Ok(())
        })
        .unwrap();
    let retiring = service.reserve_run(&task.id).unwrap();
    retiring.set_resume_supported(true);
    retiring.mark_idle();
    assert!(retiring.try_retire_idle(
        Instant::now() + Duration::from_secs(6),
        Duration::from_secs(5),
    ));

    let worker = {
        let service = service.clone();
        let task_id = task.id.clone();
        thread::spawn(move || {
            service.deliver_accepted(
                task_id,
                AcceptedTurn {
                    receipt: "old".into(),
                    prompt: "old prompt".into(),
                },
            )
        })
    };
    // Stop removes the sole receipt while the worker is blocked in
    // `available_runtime`. A subsequent accepted turn is deliberately
    // modelled before retirement release; the old delivery must not attach
    // its prompt to that newer generation.
    service.cancel(&task.id).unwrap();
    service
        .mutate_data(None, |data| {
            data.snapshot
                .tasks
                .iter_mut()
                .find(|value| value.id == task.id)
                .unwrap()
                .status = "running".into();
            data.accepted_turns.insert(
                task.id.clone(),
                AcceptedTurn {
                    receipt: "new".into(),
                    prompt: "new prompt".into(),
                },
            );
            Ok(())
        })
        .unwrap();
    service.release_run_if_current(&task.id, &retiring);
    retiring.finish_retirement();
    worker.join().unwrap();
    assert!(service.runs.lock().unwrap().tasks.get(&task.id).is_none());
    assert_eq!(
        service.data.lock().unwrap().accepted_turns[&task.id].receipt,
        "new"
    );
    service.cleanup();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn waiting_receipt_starts_once_after_retiring_owner_releases() {
    let (service, task, root) = service_and_task();
    let fixture = root.join("receipt-fixture.sh");
    std::fs::write(&fixture, "#!/bin/sh\nread ignored\nsleep 2\n").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        &fixture,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let accepted = service
        .mutate_data(None, |data| {
            let task = data
                .snapshot
                .tasks
                .iter_mut()
                .find(|value| value.id == task.id)
                .unwrap();
            task.provider = "claude".into();
            task.sandbox = "harness-configured".into();
            task.status = "running".into();
            data.task_hosts.get_mut(&task.id).unwrap().claude_path = fixture.display().to_string();
            let accepted = AcceptedTurn {
                receipt: "wait".into(),
                prompt: "one".into(),
            };
            data.accepted_turns
                .insert(task.id.clone(), accepted.clone());
            Ok(accepted)
        })
        .unwrap();
    let retiring = service.reserve_run(&task.id).unwrap();
    retiring.set_resume_supported(true);
    retiring.mark_idle();
    assert!(retiring.try_retire_idle(
        Instant::now() + Duration::from_secs(6),
        Duration::from_secs(5)
    ));
    let worker = {
        let service = service.clone();
        let task_id = task.id.clone();
        thread::spawn(move || service.deliver_accepted(task_id, accepted))
    };
    service.release_run_if_current(&task.id, &retiring);
    retiring.finish_retirement();
    worker.join().unwrap();
    let replacement = (0..50)
        .find_map(|_| {
            let owner = service.runs.lock().unwrap().tasks.get(&task.id).cloned();
            if owner
                .as_ref()
                .is_some_and(|owner| !Arc::ptr_eq(owner, &retiring))
            {
                owner
            } else {
                thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("receipt should start one replacement owner");
    assert!(service
        .data
        .lock()
        .unwrap()
        .accepted_turns
        .get(&task.id)
        .is_none());
    replacement.cancel();
    service.cleanup();
    let _ = std::fs::remove_dir_all(root);
}
