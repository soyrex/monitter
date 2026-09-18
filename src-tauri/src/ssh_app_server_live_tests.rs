//! Opt-in SSH handshake only: no model turn, native thread or production state.
use crate::{model::*, runner};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read},
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

struct Stop(Arc<runner::RunControl>);
impl Drop for Stop {
    fn drop(&mut self) {
        self.0.terminate_owned();
    }
}

#[test]
#[ignore = "Requires MONITTER_LIVE_SSH_CODEX=1 and an already trusted SSH host; no model turn"]
fn live_ssh_codex_app_server_handshake() {
    assert_eq!(std::env::var("MONITTER_LIVE_SSH_CODEX").as_deref(), Ok("1"));
    let required = |name: &str| std::env::var(name).expect(name);
    let host = Host {
        id: id(),
        name: "Isolated SSH handshake".into(),
        kind: "ssh".into(),
        address: required("MONITTER_TEST_SSH_HOST"),
        user: String::new(),
        port: 0,
        identity_file: String::new(),
        default_cwd: required("MONITTER_TEST_SSH_CWD"),
        codex_path: required("MONITTER_TEST_SSH_CODEX"),
        claude_path: String::new(),
        opencode_path: String::new(),
        hermes_path: String::new(),
    };
    let task = Task {
        id: id(),
        agent_id: id(),
        title: "SSH handshake only".into(),
        native_session_id: None,
        status: "running".into(),
        archived: false,
        created_at: now(),
        updated_at: now(),
        parent_task_id: None,
        channel_id: None,
        host_id: host.id.clone(),
        cwd: host.default_cwd.clone(),
        provider: "codex".into(),
        model: String::new(),
        model_settings: None,
        sandbox: "read-only".into(),
        project_id: None,
        acp: None,
        archived_agent_name: None,
        codex_home: None,
    };
    let control = runner::RunControl::new(true);
    let _stop = Stop(control.clone());
    let runner::SpawnedAppServer {
        mut child,
        stderr,
        cwd,
        ..
    } = runner::spawn_codex_app_server(&host, &task, None, &control, None).expect("SSH startup failed");
    assert!(cwd.starts_with('/'), "Remote cwd must be absolute");
    let stdout = child.stdout.take().expect("SSH stdout");
    let stdin = child.stdin.take().expect("SSH stdin");
    if let Some(stderr) = stderr {
        // Drain diagnostics without exposing the remote host's configuration.
        thread::spawn(move || {
            let _ = std::io::copy(&mut stderr.take(64 * 1024), &mut std::io::sink());
        });
    }
    if let Err((mut child, _)) = control.install(child, Some(stdin)) {
        runner::terminate_bounded(&mut child);
        panic!("Could not own the SSH child");
    }
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout)
            .take(2 * 1024 * 1024)
            .read_line(&mut line)
            .map(|_| line);
        let _ = tx.send(result);
    });
    control
        .send_control(
            &json!({
                "id":1,"method":"initialize","params":{
                    "clientInfo":{"name":"MonitterSshHandshake","version":"0.1"},
                    "capabilities":{"experimentalApi":false}
                }
            })
            .to_string(),
        )
        .unwrap();
    let line = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("Initialize timed out")
        .unwrap();
    let value: Value = serde_json::from_str(&line).expect("Invalid initialize JSON");
    assert_eq!(value["id"], 1);
    assert!(value.get("error").is_none(), "Initialize rejected");
    assert!(value
        .pointer("/result/userAgent")
        .and_then(Value::as_str)
        .is_some());
    control
        .send_control(&json!({"method":"initialized"}).to_string())
        .unwrap();
    control.terminate_owned();
}
