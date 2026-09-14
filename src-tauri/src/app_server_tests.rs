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

    struct FixtureCleanup {
        service: Option<Arc<Service>>,
        task_id: String,
        state_dir: PathBuf,
        executable: PathBuf,
    }

    impl Drop for FixtureCleanup {
        fn drop(&mut self) {
            if let Some(service) = self.service.take() {
                let control = service
                    .runs
                    .lock()
                    .ok()
                    .and_then(|runs| runs.tasks.get(&self.task_id).cloned());
                if let Some(control) = control {
                    control.terminate_owned();
                }
            }
            let _ = std::fs::remove_dir_all(&self.state_dir);
            let _ = std::fs::remove_file(&self.executable);
        }
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

    fn resolve_fixture_requests_with_timeout(
        service: &Arc<Service>,
        task_id: &str,
        timeout: Duration,
    ) {
        for _ in 0..3 {
            wait_until(service, task_id, timeout, |snapshot| {
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

    fn resolve_fixture_requests(service: &Arc<Service>, task_id: &str) {
        resolve_fixture_requests_with_timeout(service, task_id, Duration::from_secs(5));
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
            .app_server_message(&task.id, &stale, "turn-1", "item-1", "live", None, false)
            .is_ok());
        stale.cancel();
        assert!(service
            .app_server_message(&task.id, &stale, "turn-1", "item-1", "stale", None, true)
            .is_err());
        let _ = std::fs::remove_dir_all(service.runtime_dir.clone());
    }

    #[test]
    fn restart_resumes_saved_thread_once_with_required_collaboration_helper() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/fixtures/codex-app-server/mock.mjs");
        let executable = std::env::temp_dir().join(format!(
            "monitter-app-server-restart-fixture-{}",
            crate::id()
        ));
        let script = format!(
            "#!/bin/sh\nMONITTER_FIXTURE_DELAY_THREAD_RESUME_MS=21000 exec /usr/bin/env node {}\n",
            crate::runner::posix_quote(&root.to_string_lossy())
        );
        std::fs::write(&executable, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let state_dir = dir("restart-resume");
        let service = Service::open(None, state_dir.clone()).unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        service
            .mutate(None, |snapshot| {
                snapshot.agents[0].collaboration_enabled = true;
                Ok(())
            })
            .unwrap();
        let task = service
            .create_task(CreateTaskInput {
                agent_id,
                title: "restart fixture".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        let mut cleanup = FixtureCleanup {
            service: Some(service.clone()),
            task_id: task.id.clone(),
            state_dir: state_dir.clone(),
            executable: executable.clone(),
        };
        service
            .mutate_data(None, |data| {
                data.task_hosts.get_mut(&task.id).unwrap().codex_path =
                    executable.to_string_lossy().into();
                Ok(())
            })
            .unwrap();
        service
            .send_fast(task.id.clone(), "first prompt".into(), vec![])
            .unwrap();
        resolve_fixture_requests(&service, &task.id);
        wait_until(&service, &task.id, Duration::from_secs(5), |snapshot| {
            snapshot
                .tasks
                .iter()
                .any(|item| item.id == task.id && item.status == "completed")
        });
        let first = service.snapshot().unwrap();
        let native = first
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .unwrap()
            .native_session_id
            .clone()
            .unwrap();
        assert_eq!(native, "00000000-0000-7000-8000-000000000001");
        assert_eq!(
            first
                .messages
                .iter()
                .filter(|item| item.task_id == task.id && item.role == "assistant")
                .count(),
            1
        );
        service.cancel(&task.id).unwrap();
        let stop_deadline = Instant::now() + Duration::from_secs(5);
        while service.runs.lock().unwrap().tasks.contains_key(&task.id) {
            assert!(
                Instant::now() < stop_deadline,
                "fixture process did not stop before restart"
            );
            thread::sleep(Duration::from_millis(20));
        }
        drop(service);
        cleanup.service = None;

        let reopened = Service::open(None, state_dir.clone()).unwrap();
        cleanup.service = Some(reopened.clone());
        reopened
            .send_fast(task.id.clone(), "second prompt".into(), vec![])
            .unwrap();
        resolve_fixture_requests_with_timeout(&reopened, &task.id, Duration::from_secs(35));
        wait_until(&reopened, &task.id, Duration::from_secs(35), |snapshot| {
            snapshot
                .messages
                .iter()
                .filter(|item| item.task_id == task.id && item.role == "assistant")
                .count()
                == 2
        });
        let resumed = reopened.snapshot().unwrap();
        let task_after = resumed
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .unwrap();
        assert_eq!(
            task_after.native_session_id.as_deref(),
            Some(native.as_str())
        );
        let user_prompts = resumed
            .messages
            .iter()
            .filter(|item| item.task_id == task.id && item.role == "user")
            .collect::<Vec<_>>();
        assert_eq!(
            user_prompts
                .iter()
                .filter(|item| item.text == "first prompt")
                .count(),
            1
        );
        assert_eq!(
            user_prompts
                .iter()
                .filter(|item| item.text == "second prompt")
                .count(),
            1
        );
        assert_eq!(user_prompts.len(), 2);
        let control = reopened.resident_control(&task.id).unwrap().unwrap();
        control.terminate_owned();
    }

    #[test]
    fn same_item_updates_stream_without_duplicate_message() {
        let service = Service::open(None, dir("dedupe")).unwrap();
        let task = running_task(&service, "/bin/echo");
        let control = service.reserve_run(&task.id).unwrap();
        control.set_app_server_thread("thread-2".into());
        control.set_app_server_turn("turn-2".into());
        service
            .app_server_message(
                &task.id,
                &control,
                "turn-2",
                "turn-2:item-2",
                "one",
                Some("final_answer"),
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
                Some("final_answer"),
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
                Some("final_answer"),
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
        assert_eq!(messages[0].phase.as_deref(), Some("final_answer"));
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
                    raw_input: None,
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
                    raw_input: None,
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
                None,
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
