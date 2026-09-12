//! Focused tests for the app-server service boundary.  These tests are kept in
//! a separate module so the protocol fixture can be used without live Codex.

#[cfg(test)]
mod tests {
    use crate::{model::*, ApprovalDecision, CreateApprovalRequest, Parsed, Service};
    use serde_json::json;
    use std::{
        path::PathBuf,
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    fn dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("monitter-app-server-{name}-{}", crate::id()))
    }

    fn running_task(service: &Arc<Service>, fixture: &str) -> Task {
        let agent = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(CreateTaskInput {
                agent_id: agent,
                title: "fixture".into(),
                native_session_id: None,
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
                data.task_hosts.get_mut(&task.id).unwrap().codex_path = fixture.into();
                data.snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task.id)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        task
    }

    fn wait_until(
        service: &Arc<Service>,
        task_id: &str,
        timeout: Duration,
        f: impl Fn(&Snapshot) -> bool,
    ) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if f(&service.snapshot().unwrap()) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for app-server fixture state for {task_id}");
    }

    fn resolve_fixture_requests(service: &Arc<Service>, task_id: &str) {
        for _ in 0..3 {
            wait_until(service, task_id, Duration::from_secs(5), |snapshot| {
                snapshot
                    .approval_requests
                    .iter()
                    .any(|request| request.task_id == task_id && request.status == "pending")
            });
            let pending = service
                .snapshot()
                .unwrap()
                .approval_requests
                .into_iter()
                .find(|request| request.task_id == task_id && request.status == "pending");
            let Some(request) = pending else { return };
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

    #[test]
    fn app_server_fixture_round_trip_persists_stream_and_interaction_records() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/fixtures/codex-app-server/mock.mjs");
        let executable =
            std::env::temp_dir().join(format!("monitter-app-server-fixture-{}", crate::id()));
        std::fs::copy(&root, &executable).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let service = Service::open(None, dir("round-trip")).unwrap();
        let task = running_task(&service, executable.to_str().unwrap());
        service
            .launch(task.id.clone(), "hello fixture".into())
            .unwrap();

        // Resolve each durable request as it arrives, preserving the same
        // service path used by the UI rather than writing to the child stdin.
        wait_until(&service, &task.id, Duration::from_secs(5), |snapshot| {
            snapshot
                .approval_requests
                .iter()
                .any(|request| request.task_id == task.id)
        });
        resolve_fixture_requests(&service, &task.id);
        wait_until(&service, &task.id, Duration::from_secs(5), |snapshot| {
            snapshot
                .tasks
                .iter()
                .find(|item| item.id == task.id)
                .is_some_and(|item| item.status == "completed")
        });
        let snapshot = service.snapshot().unwrap();
        let native = snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .unwrap()
            .native_session_id
            .clone();
        assert_eq!(
            snapshot
                .messages
                .iter()
                .filter(|message| message.task_id == task.id && message.role == "assistant")
                .count(),
            1
        );
        assert!(snapshot
            .approval_requests
            .iter()
            .any(|request| request.input.is_some()));
        let control = service.resident_control(&task.id).unwrap().unwrap();
        service
            .send_fast(task.id.clone(), "second fixture".into(), vec![])
            .unwrap();
        resolve_fixture_requests(&service, &task.id);
        wait_until(&service, &task.id, Duration::from_secs(5), |snapshot| {
            snapshot
                .messages
                .iter()
                .filter(|message| message.task_id == task.id && message.role == "assistant")
                .count()
                == 2
        });
        wait_until(&service, &task.id, Duration::from_secs(5), |snapshot| {
            snapshot
                .tasks
                .iter()
                .find(|item| item.id == task.id)
                .is_some_and(|item| item.status == "completed")
        });
        let second = service.snapshot().unwrap();
        assert_eq!(
            second
                .tasks
                .iter()
                .find(|item| item.id == task.id)
                .unwrap()
                .native_session_id,
            native
        );
        assert_eq!(
            service
                .resident_control(&task.id)
                .unwrap()
                .map(|current| Arc::as_ptr(&current)),
            Some(Arc::as_ptr(&control))
        );
        control.terminate_owned();
        let _ = std::fs::remove_dir_all(service.runtime_dir.clone());
        let _ = std::fs::remove_file(executable);
    }

    #[test]
    fn stale_app_server_turn_cannot_mutate_replacement_run() {
        let service = Service::open(None, dir("stale-turn")).unwrap();
        let task = running_task(&service, "/bin/echo");
        let stale = service.reserve_run(&task.id).unwrap();
        stale.set_app_server_thread("thread-1".into());
        stale.set_app_server_turn("turn-1".into());
        assert!(service
            .app_server_message(&task.id, &stale, "turn-1", "item-1", "live", false)
            .is_ok());
        stale.cancel();
        assert!(service
            .app_server_message(&task.id, &stale, "turn-1", "item-1", "stale", true)
            .is_err());
        let _ = std::fs::remove_dir_all(service.runtime_dir.clone());
    }

    #[test]
    fn same_item_updates_stream_without_duplicate_message() {
        let service = Service::open(None, dir("dedupe")).unwrap();
        let task = running_task(&service, "/bin/echo");
        let control = service.reserve_run(&task.id).unwrap();
        control.set_app_server_thread("thread-2".into());
        control.set_app_server_turn("turn-2".into());
        service
            .app_server_message(&task.id, &control, "turn-2", "turn-2:item-2", "one", false)
            .unwrap();
        service
            .app_server_message(
                &task.id,
                &control,
                "turn-2",
                "turn-2:item-2",
                "one two",
                false,
            )
            .unwrap();
        service
            .app_server_message(
                &task.id,
                &control,
                "turn-2",
                "turn-2:item-2",
                "one two",
                true,
            )
            .unwrap();
        let messages = service
            .snapshot()
            .unwrap()
            .messages
            .into_iter()
            .filter(|m| m.task_id == task.id && m.role == "assistant")
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "one two");
        assert_eq!(messages[0].stream_status.as_deref(), Some("complete"));
        let _ = std::fs::remove_dir_all(service.runtime_dir.clone());
    }

    #[test]
    fn cancellation_expires_pending_app_server_approval_and_blocks_response() {
        let service = Service::open(None, dir("cancel-approval")).unwrap();
        let task = running_task(&service, "/bin/echo");
        let control = service.reserve_run(&task.id).unwrap();
        control.set_app_server_thread("thread-3".into());
        control.set_app_server_turn("turn-3".into());
        let request = service
            .create_app_server_approval(
                &control,
                "turn-3",
                CreateApprovalRequest {
                    task_id: task.id.clone(),
                    provider: "codex".into(),
                    run_id: "rpc:3".into(),
                    tool: "Command execution".into(),
                    summary: "Run fixture".into(),
                    detail: "fixture".into(),
                    risk: "high".into(),
                },
                None,
            )
            .unwrap();
        service.cancel(&task.id).unwrap();
        assert!(service
            .resolve_approval_request(&request.id, ApprovalDecision::ApproveOnce)
            .is_err());
        assert_eq!(
            service.snapshot().unwrap().approval_requests[0].status,
            "expired"
        );
        let _ = std::fs::remove_dir_all(service.runtime_dir.clone());
    }

    #[test]
    fn denial_is_persisted_and_generated_image_is_attached_to_final_message() {
        let service = Service::open(None, dir("deny-image")).unwrap();
        let task = running_task(&service, "/bin/echo");
        let control = service.reserve_run(&task.id).unwrap();
        control.set_app_server_thread("thread-image".into());
        control.set_app_server_turn("turn-image".into());
        let request = service
            .create_app_server_approval(
                &control,
                "turn-image",
                CreateApprovalRequest {
                    task_id: task.id.clone(),
                    provider: "codex".into(),
                    run_id: "rpc:image".into(),
                    tool: "Command execution".into(),
                    summary: "Run fixture".into(),
                    detail: "fixture".into(),
                    risk: "high".into(),
                },
                None,
            )
            .unwrap();
        service
            .resolve_approval_request(&request.id, ApprovalDecision::Deny)
            .unwrap();
        assert_eq!(
            service.snapshot().unwrap().approval_requests[0].status,
            "denied"
        );
        service
            .app_server_event(
                &task.id,
                &control,
                Some("turn-image"),
                Parsed {
                    native_session_id: None,
                    assistant: None,
                    event: Some((
                        "computer_image".into(),
                        "MCP image result".into(),
                        "/9j/2Q==".into(),
                    )),
                    failed: false,
                },
            )
            .unwrap();
        service
            .app_server_message(
                &task.id,
                &control,
                "turn-image",
                "turn-image:item",
                "image reply",
                true,
            )
            .unwrap();
        let message = service
            .snapshot()
            .unwrap()
            .messages
            .into_iter()
            .find(|item| item.task_id == task.id && item.role == "assistant")
            .unwrap();
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(message.attachments[0].mime_type, "image/jpeg");
        control.terminate_owned();
        let _ = std::fs::remove_dir_all(service.runtime_dir.clone());
    }
}
