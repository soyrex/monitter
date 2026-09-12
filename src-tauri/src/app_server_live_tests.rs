//! Opt-in smoke test with the installed Codex account. Never runs in the default suite.
use crate::*;

struct Cleanup(Arc<Service>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let controls = self
            .0
            .runs
            .lock()
            .unwrap()
            .tasks
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for control in controls {
            control.terminate_owned();
        }
    }
}

#[test]
#[ignore = "Uses the existing Codex account; explicitly opt in with MONITTER_LIVE_CODEX=1"]
fn live_codex_app_server_denial_prevents_scratch_write() {
    assert_eq!(std::env::var("MONITTER_LIVE_CODEX").as_deref(), Ok("1"));
    let directory = std::env::temp_dir().join(format!("monitter-live-approval-{}", id()));
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("denied-test.txt");
    let service = Service::open(None, directory.join("state")).unwrap();
    let _cleanup = Cleanup(service.clone());
    service.mutate(None, |snapshot| {
        let agent = &mut snapshot.agents[0];
        agent.cwd = directory.to_string_lossy().into_owned();
        agent.model = "gpt-5.6-luna".into();
        agent.sandbox = "read-only".into();
        agent.collaboration_enabled = false;
        agent.instructions = "You are testing a command-approval interface. Execute only the exact shell command the user requests. If denied, stop and report the denial; do not retry or use another tool.".into();
        Ok(())
    }).unwrap();
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let task = service
        .create_task(CreateTaskInput {
            agent_id,
            title: "Isolated approval-denial test".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: Some(directory.to_string_lossy().into_owned()),
            model_settings: None,
            sandbox: Some("read-only".into()),
        })
        .unwrap();
    let command = format!(
        "printf 'monitter-approval-test' > {}",
        runner::posix_quote(&output.to_string_lossy())
    );
    service.send_fast(task.id.clone(), format!("Test the approval UI by calling exec_command with this exact command: {command}\nSet sandbox_permissions to require_escalated and justification to 'May I create this isolated test file?'. Do not run it without requesting approval first. If approval is denied, stop and report that; never retry or choose another tool."), vec![]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let request = loop {
        let snapshot = service.snapshot().unwrap();
        if let Some(request) = snapshot
            .approval_requests
            .iter()
            .find(|r| r.task_id == task.id && r.status == "pending")
        {
            break request.clone();
        }
        assert_eq!(
            snapshot
                .tasks
                .iter()
                .find(|t| t.id == task.id)
                .unwrap()
                .status,
            "running",
            "Codex ended without requesting approval: {:?}",
            snapshot
                .events
                .iter()
                .filter(|e| e.kind == "error")
                .collect::<Vec<_>>()
        );
        assert!(Instant::now() < deadline, "Real approval did not arrive");
        thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(request.tool, "Command execution");
    assert!(request.detail.contains("denied-test.txt"));
    assert!(!output.exists());
    service
        .resolve_approval_request(&request.id, ApprovalDecision::Deny)
        .unwrap();
    loop {
        let snapshot = service.snapshot().unwrap();
        let status = &snapshot
            .tasks
            .iter()
            .find(|t| t.id == task.id)
            .unwrap()
            .status;
        if status != "running" {
            assert_eq!(status, "completed");
            assert_eq!(
                snapshot
                    .approval_requests
                    .iter()
                    .find(|r| r.id == request.id)
                    .unwrap()
                    .status,
                "denied"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Codex did not finish after denial"
        );
        thread::sleep(Duration::from_millis(100));
    }
    assert!(!output.exists(), "Denied command created a file");
    eprintln!(
        "Real Codex approval-denial round trip passed: {}",
        directory.display()
    );
}

#[test]
#[ignore = "Uses the existing Codex account; explicitly opt in with MONITTER_LIVE_CODEX=1"]
fn live_codex_app_server_two_turns_preserve_context() {
    assert_eq!(std::env::var("MONITTER_LIVE_CODEX").as_deref(), Ok("1"));
    let directory = std::env::temp_dir().join(format!("monitter-live-app-server-{}", id()));
    std::fs::create_dir_all(&directory).unwrap();
    let service = Service::open(None, directory.join("state")).unwrap();
    let _cleanup = Cleanup(service.clone());
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    service.mutate(None, |snapshot| {
        let agent = &mut snapshot.agents[0];
        agent.cwd = directory.to_string_lossy().into_owned();
        agent.model = "gpt-5.6-luna".into();
        agent.sandbox = "read-only".into();
        agent.collaboration_enabled = false;
        agent.instructions = "You are testing Monitter's Codex adapter. Reply briefly; do not use any tools or make any changes.".into();
        Ok(())
    }).unwrap();
    let task = service
        .create_task(CreateTaskInput {
            agent_id,
            title: "Monitter app-server integration test".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: Some(directory.to_string_lossy().into_owned()),
            model_settings: None,
            sandbox: Some("read-only".into()),
        })
        .unwrap();
    let token = format!("monitter-{}", id());
    let wait = || {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let snapshot = service.snapshot().unwrap();
            let task = snapshot.tasks.iter().find(|t| t.id == task.id).unwrap();
            if task.status == "completed" {
                return snapshot;
            }
            assert_eq!(
                task.status,
                "running",
                "Live Codex failed: {:?}",
                snapshot
                    .events
                    .iter()
                    .filter(|e| e.kind == "error")
                    .collect::<Vec<_>>()
            );
            assert!(Instant::now() < deadline, "Live Codex timed out");
            thread::sleep(Duration::from_millis(100));
        }
    };
    service.send_fast(task.id.clone(), format!("Remember this token for my next message: {token}. Reply exactly READY. Do not use tools."), vec![]).unwrap();
    let first = wait();
    assert!(first
        .messages
        .iter()
        .any(|m| m.task_id == task.id && m.role == "assistant" && m.text.trim() == "READY"));
    let native = first
        .tasks
        .iter()
        .find(|t| t.id == task.id)
        .unwrap()
        .native_session_id
        .clone()
        .expect("Native thread ID");
    let control = service.runs.lock().unwrap().tasks[&task.id].clone();
    service
        .send_fast(
            task.id.clone(),
            "What token did I give you? Reply with only that token. Do not use tools.".into(),
            vec![],
        )
        .unwrap();
    let second = wait();
    assert_eq!(
        second
            .tasks
            .iter()
            .find(|t| t.id == task.id)
            .unwrap()
            .native_session_id
            .as_deref(),
        Some(native.as_str())
    );
    assert!(Arc::ptr_eq(
        &control,
        &service.runs.lock().unwrap().tasks[&task.id]
    ));
    assert!(second
        .messages
        .iter()
        .any(|m| m.role == "assistant" && m.text.trim() == token));
    assert_eq!(
        second
            .messages
            .iter()
            .filter(|m| m.role == "assistant")
            .count(),
        2
    );
    // Stop the resident process, then bootstrap a replacement using the saved
    // native thread. This must preserve context without replaying either turn.
    service.cancel(&task.id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while service.runs.lock().unwrap().tasks.contains_key(&task.id) {
        assert!(
            Instant::now() < deadline,
            "Stopped Codex process was not released"
        );
        thread::sleep(Duration::from_millis(50));
    }
    service.send_fast(task.id.clone(), "After restarting the connection, what token did I give you? Reply with only the token. Do not use tools.".into(), vec![]).unwrap();
    let resumed = wait();
    assert_eq!(
        resumed
            .tasks
            .iter()
            .find(|t| t.id == task.id)
            .unwrap()
            .native_session_id
            .as_deref(),
        Some(native.as_str())
    );
    assert!(!Arc::ptr_eq(
        &control,
        &service.runs.lock().unwrap().tasks[&task.id]
    ));
    let replies = resumed
        .messages
        .iter()
        .filter(|m| m.role == "assistant")
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies.last().unwrap().text.trim(), token);
    eprintln!(
        "Live Codex resident-turn and Stop/resume smoke passed; isolated test state: {}",
        directory.display()
    );
}
