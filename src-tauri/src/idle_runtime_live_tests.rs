//! Explicit opt-in native CLI check; never runs as part of the normal suite.
use crate::*;

struct Cleanup(Arc<Service>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.0.cleanup();
        let owners = self
            .0
            .runs
            .lock()
            .unwrap()
            .tasks
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for owner in owners {
            owner.terminate_owned();
        }
    }
}

fn completed(service: &Service, task_id: &str) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let snapshot = service.snapshot().unwrap();
        let task = snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .unwrap();
        if task.status == "completed" {
            // Completion is persisted just before the runtime idle transition.
            if service
                .runs
                .lock()
                .unwrap()
                .tasks
                .get(task_id)
                .is_some_and(|owner| owner.is_idle())
            {
                return snapshot;
            }
        } else {
            assert_eq!(
                task.status,
                "running",
                "Native test failed: {:?}",
                snapshot
                    .events
                    .iter()
                    .filter(|event| event.kind == "error")
                    .collect::<Vec<_>>()
            );
        }
        assert!(Instant::now() < deadline, "Native idle test timed out");
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "Uses existing native account; set MONITTER_LIVE_IDLE_PROVIDER=codex or opencode-acp"]
fn live_idle_retirement_preserves_native_context_and_releases_memory() {
    let provider =
        std::env::var("MONITTER_LIVE_IDLE_PROVIDER").expect("explicit native provider opt-in");
    assert!(matches!(provider.as_str(), "codex" | "opencode-acp"));
    let root = std::env::temp_dir().join(format!("monitter-live-idle-{}", id()));
    std::fs::create_dir_all(&root).unwrap();
    let service = Service::open(None, root.join("state")).unwrap();
    let _cleanup = Cleanup(service.clone());
    service.mutate(None, |snapshot| {
        let agent = &mut snapshot.agents[0];
        agent.cwd = root.to_string_lossy().into_owned();
        agent.collaboration_enabled = false;
        agent.instructions = "You are testing native conversation continuity. Do not use tools. Follow the requested exact reply format.".into();
        if provider == "codex" {
            agent.provider = "codex".into();
            agent.model = "gpt-5.6-luna".into();
            agent.sandbox = "read-only".into();
        } else {
            agent.provider = "acp".into();
            agent.model.clear();
            agent.sandbox = "harness-configured".into();
            agent.acp = Some(AcpLaunch {
                command: runner::resolve_local_provider("opencode", "")?.to_string_lossy().into_owned(),
                args: vec!["acp".into()],
            });
        }
        Ok(())
    }).unwrap();
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let task = service
        .create_task(CreateTaskInput {
            agent_id,
            title: "Isolated idle retirement smoke".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: Some(root.to_string_lossy().into_owned()),
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let token = format!("orchard-{}", id());
    service
        .send_fast(
            task.id.clone(),
            format!("Remember this test token: {token}. Reply exactly READY. Do not use tools."),
            vec![],
        )
        .unwrap();
    let first = completed(&service, &task.id);
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
        .unwrap();
    let old = service.runs.lock().unwrap().tasks[&task.id].clone();
    let before = process_metrics::sample().unwrap();
    eprintln!("Before live retirement: {}", old.retirement_diagnostic());
    // Advancing the idle clock does not fast-forward native helper startup.
    // A helper born between ownership samples safely skips that sweep; allow
    // later sweeps to establish ownership once initialization settles.
    let retirement_deadline = Instant::now() + Duration::from_secs(30);
    let retired = loop {
        let retired = service.collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301));
        if retired == 1 || Instant::now() >= retirement_deadline {
            break retired;
        }
        thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(
        retired,
        1,
        "Runtime was not safely collectible: {}",
        old.retirement_diagnostic()
    );
    for process in before.processes.iter().filter(|p| p.pid != before.root_pid) {
        assert!(
            !process_metrics::retirement_process_tree(process.pid)
                .is_ok_and(|tree| tree.contains(&(process.pid, process.started_at))),
            "Retired helper is still alive (including reparented processes): {} {}",
            process.pid,
            process.name,
        );
    }
    assert_eq!(
        first,
        service.snapshot().unwrap(),
        "GC must leave the visible chat unchanged"
    );
    let after = process_metrics::sample().unwrap();
    let released = before
        .processes
        .iter()
        .filter(|p| {
            p.pid != before.root_pid
                && !after
                    .processes
                    .iter()
                    .any(|a| a.pid == p.pid && a.started_at == p.started_at)
        })
        .map(|p| p.resident_memory_bytes)
        .sum::<u64>();
    assert!(released > 0, "Expected native process RAM to be released");
    let wake_started = Instant::now();
    service
        .send_fast(
            task.id.clone(),
            "What test token did I give you? Reply only with that token. Do not use tools.".into(),
            vec![],
        )
        .unwrap();
    let resumed = completed(&service, &task.id);
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
    assert!(resumed
        .messages
        .iter()
        .any(|m| m.task_id == task.id && m.role == "assistant" && m.text.trim() == token));
    assert!(!Arc::ptr_eq(
        &old,
        &service.runs.lock().unwrap().tasks[&task.id]
    ));
    eprintln!("IDLE_GC_NATIVE provider={provider} released_rss_bytes={released} wake_to_reply_ms={} native_session_preserved=true state={}", wake_started.elapsed().as_millis(), root.display());
}
