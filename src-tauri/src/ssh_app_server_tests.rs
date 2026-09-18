//! Exercises the production adapter and remote bootstrap through a local SSH shim.
use crate::{model::*, runner, ApprovalDecision, Service};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    service: Arc<Service>,
    task: Task,
    directory: PathBuf,
    _ssh: runner::TestSshOverride,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.service.run_is_active(&self.task.id) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        // Only scratch data created by this fixture, after owned processes stop.
        if !self.service.run_is_active(&self.task.id) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}
impl Fixture {
    fn new(collaboration: bool, fail_host_key: bool) -> Self {
        let directory = std::env::temp_dir().join(format!("monitter-ssh-test-{}", crate::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/fixtures/codex-app-server");
        for name in ["ssh.py", "ssh-inspect.mjs"] {
            std::fs::copy(source.join(name), directory.join(name)).unwrap();
        }
        let mock = std::fs::read_to_string(source.join("mock.mjs")).unwrap();
        std::fs::write(
            directory.join("mock.mjs"),
            mock.replacen(
                "// Deterministic",
                "import './ssh-inspect.mjs';\n// Deterministic",
                1,
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["ssh.py", "mock.mjs"] {
                std::fs::set_permissions(
                    directory.join(name),
                    std::fs::Permissions::from_mode(0o700),
                )
                .unwrap();
            }
        }
        let cwd = directory.join("project with 'quotes'");
        std::fs::create_dir(&cwd).unwrap();
        let service = Service::open(None, directory.join("state")).unwrap();
        let host_id = service
            .mutate(None, |snapshot| {
                let host = &mut snapshot.hosts[0];
                host.kind = "ssh".into();
                host.address = if fail_host_key {
                    "hostkey-failure"
                } else {
                    "fixture.invalid"
                }
                .into();
                host.codex_path = directory.join("mock.mjs").to_string_lossy().into();
                host.default_cwd = cwd.to_string_lossy().into();
                let agent = &mut snapshot.agents[0];
                agent.host_id = host.id.clone();
                agent.cwd = host.default_cwd.clone();
                agent.model.clear();
                agent.collaboration_enabled = collaboration;
                Ok(host.id.clone())
            })
            .unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(CreateTaskInput {
                agent_id,
                title: "SSH fixture".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: Some("read-only".into()),
            })
            .unwrap();
        let ssh = runner::override_ssh_for_test(&host_id, &directory.join("ssh.py"));
        Self {
            service,
            task,
            directory,
            _ssh: ssh,
        }
    }
    fn wait(&self, test: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let snapshot = self.service.snapshot().unwrap();
            if test(&snapshot) {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "SSH fixture timed out: {:?}",
                snapshot
                    .events
                    .iter()
                    .filter(|e| e.kind == "error")
                    .collect::<Vec<_>>()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn send(&self, text: &str) {
        self.service
            .send_fast(self.task.id.clone(), text.into(), vec![])
            .unwrap();
    }
    fn resolve_turn(&self) {
        for _ in 0..3 {
            let snapshot = self.wait(|s| {
                s.approval_requests
                    .iter()
                    .any(|r| r.task_id == self.task.id && r.status == "pending")
            });
            let request = snapshot
                .approval_requests
                .iter()
                .find(|r| r.task_id == self.task.id && r.status == "pending")
                .unwrap();
            if request.input.is_some() {
                self.service
                    .resolve_input_request(
                        &request.id,
                        json!({"answers":{"confirm":{"answers":["yes"]}}}),
                    )
                    .unwrap();
            } else {
                self.service
                    .resolve_approval_request(&request.id, ApprovalDecision::ApproveOnce)
                    .unwrap();
            }
        }
        self.wait(|s| {
            s.tasks
                .iter()
                .any(|t| t.id == self.task.id && t.status == "completed")
        });
    }
    fn rpc(&self) -> Vec<Value> {
        std::fs::read_to_string(self.directory.join("rpc.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn stop(&self) {
        // Public Stop expires pending approvals on a running turn. A completed
        // turn keeps its resident transport, so end that fixture-owned process.
        match self.service.cancel(&self.task.id) {
            Ok(_) => {}
            Err(error) if error == "Task is not running." => {
                if let Some(control) = self.service.resident_control(&self.task.id).unwrap() {
                    control.cancel();
                }
            }
            Err(error) => panic!("Could not stop SSH fixture: {error}"),
        }
        self.wait(|_| !self.service.run_is_active(&self.task.id));
    }
}

#[test]
fn ssh_codex_dispatches_app_server_and_reuses_resident_process() {
    let f = Fixture::new(true, false);
    f.send("first");
    f.resolve_turn();
    let original = f.service.resident_control(&f.task.id).unwrap().unwrap();
    let token = f.service.collaboration_grants.lock().unwrap()[&f.task.id]
        .token
        .clone();
    f.send("second");
    f.resolve_turn();
    assert!(Arc::ptr_eq(
        &original,
        &f.service.resident_control(&f.task.id).unwrap().unwrap()
    ));
    let snapshot = f.service.snapshot().unwrap();
    assert_eq!(
        snapshot
            .messages
            .iter()
            .filter(|m| m.task_id == f.task.id && m.role == "assistant")
            .count(),
        2
    );
    let rpc = f.rpc();
    assert_eq!(rpc.iter().filter(|r| r.get("pid").is_some()).count(), 1);
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "turn/start").count(),
        2
    );
    assert!(rpc.iter().any(|r| r["brokerOk"] == true));
    assert!(rpc.iter().any(|r| r["hasToken"] == true));
    let args = std::fs::read_to_string(f.directory.join("ssh-argv.jsonl")).unwrap();
    assert!(!args.contains(&token));
    assert!(!serde_json::to_string(&snapshot).unwrap().contains(&token));
    assert!(args.contains("StrictHostKeyChecking=yes"));
    let config = &rpc
        .iter()
        .find(|r| r["method"] == "thread/start")
        .unwrap()["params"]["config"];
    assert!(config["mcp_servers.monitter.url"]
        .as_str()
        .is_some_and(|url| url.starts_with("http://127.0.0.1:")));
    assert_eq!(config["mcp_servers.monitter.bearer_token_env_var"], "MONITTER_TOKEN");
    f.stop();
}

#[test]
fn ssh_codex_cancellation_expires_pending_approval_and_releases_run() {
    let f = Fixture::new(false, false);
    f.send("cancel");
    let snapshot = f.wait(|s| s.approval_requests.iter().any(|r| r.status == "pending"));
    let approval = snapshot
        .approval_requests
        .iter()
        .find(|r| r.status == "pending")
        .unwrap()
        .id
        .clone();
    f.stop();
    let snapshot = f.service.snapshot().unwrap();
    assert!(snapshot
        .approval_requests
        .iter()
        .any(|r| r.id == approval && r.status == "expired"));
    assert!(f
        .service
        .resolve_approval_request(&approval, ApprovalDecision::ApproveOnce)
        .is_err());
    assert_eq!(
        f.rpc()
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .count(),
        1
    );
}

#[test]
fn ssh_codex_resume_uses_same_native_thread_without_replaying_messages() {
    let f = Fixture::new(true, false);
    f.send("first");
    f.resolve_turn();
    let native = f.service.snapshot().unwrap().tasks[0]
        .native_session_id
        .clone();
    f.stop();
    f.send("second");
    f.resolve_turn();
    let snapshot = f.service.snapshot().unwrap();
    assert_eq!(snapshot.tasks[0].native_session_id, native);
    let rpc = f.rpc();
    assert_eq!(rpc.iter().filter(|r| r.get("pid").is_some()).count(), 2);
    assert_eq!(
        rpc.iter().filter(|r| r["method"] == "turn/start").count(),
        2
    );
    let resume = rpc.iter().find(|r| r["method"] == "thread/resume").unwrap();
    assert_eq!(resume["params"]["threadId"].as_str(), native.as_deref());
    assert_eq!(resume["params"]["excludeTurns"], true);
    assert_eq!(
        snapshot
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
}

#[test]
fn ssh_codex_reports_real_host_key_errors_with_and_without_collaboration() {
    for collaboration in [false, true] {
        let f = Fixture::new(collaboration, true);
        f.send("never delivered");
        let snapshot = f.wait(|s| s.tasks.iter().any(|t| t.status == "error"));
        assert!(snapshot
            .events
            .iter()
            .any(|e| e.kind == "error" && e.detail.contains("Host key verification failed")));
        assert!(snapshot.tasks[0].native_session_id.is_none());
        assert!(!f.directory.join("rpc.jsonl").exists());
    }
}

#[test]
#[cfg(unix)]
fn ssh_codex_process_loss_expires_approval_without_replaying_the_turn() {
    let f = Fixture::new(false, false);
    f.send("one uncertain turn");
    let snapshot = f.wait(|s| s.approval_requests.iter().any(|r| r.status == "pending"));
    let approval = snapshot
        .approval_requests
        .iter()
        .find(|r| r.status == "pending")
        .unwrap()
        .id
        .clone();
    // This PID is emitted only by our owned scratch Codex fixture, not a host
    // process search or a production agent. Abrupt death closes the SSH RPC pipe.
    let pid = f.rpc().iter().find_map(|r| r["pid"].as_i64()).unwrap() as i32;
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    let snapshot = f.wait(|s| s.tasks[0].status == "error" && !f.service.run_is_active(&f.task.id));
    assert!(snapshot
        .approval_requests
        .iter()
        .any(|r| r.id == approval && r.status == "expired"));
    assert!(f
        .service
        .resolve_approval_request(&approval, ApprovalDecision::ApproveOnce)
        .is_err());
    assert_eq!(
        f.rpc()
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .count(),
        1
    );
}
