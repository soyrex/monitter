//! ACP retirement fixtures: a deliberate EOF must wake one saved session,
//! never be mistaken for an unexpected transport failure.

use crate::{
    model::{AcpLaunch, CreateTaskInput},
    runtime_gc::IDLE_RUNTIME_TIMEOUT,
    Service,
};
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
    capture: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_file(&self.capture);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!("monitter-acp-idle-{}", crate::id()));
    let script = std::env::temp_dir().join(format!("monitter-acp-idle-{}.cjs", crate::id()));
    let capture = std::env::temp_dir().join(format!("monitter-acp-idle-{}.log", crate::id()));
    let source = r#"#!/usr/bin/env node
const fs=require('node:fs'), readline=require('node:readline'), log=process.argv[2];
const out=(id,result)=>console.log(JSON.stringify({jsonrpc:'2.0',id,result}));
const note=s=>fs.appendFileSync(log,s+'\n');
readline.createInterface({input:process.stdin}).on('line', line=>{
 const f=JSON.parse(line); note(f.method||`response:${f.id}`);
 if(f.method==='initialize') out(f.id,{protocolVersion:1,agentCapabilities:{sessionCapabilities:{resume:{}}}});
 else if(f.method==='session/new') out(f.id,{sessionId:'native-retire-id'});
 else if(f.method==='session/resume') out(f.id,{});
 else if(f.method==='session/prompt') out(f.id,{stopReason:'end_turn'});
});
"#;
    fs::write(&script, source).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let service = Service::open(None, root.clone()).unwrap();
    service
        .mutate(None, |snapshot| {
            let agent = &mut snapshot.agents[0];
            agent.provider = "acp".into();
            agent.sandbox = "harness-configured".into();
            agent.cwd = root.to_string_lossy().into_owned();
            agent.acp = Some(AcpLaunch {
                command: script.to_string_lossy().into_owned(),
                args: vec![capture.to_string_lossy().into_owned()],
            });
            Ok(())
        })
        .unwrap();
    Fixture {
        service,
        root,
        capture,
    }
}

fn wait_for(service: &Service, task_id: &str, done: impl Fn(&crate::Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let snapshot = service.snapshot().unwrap();
        if done(&snapshot) {
            return;
        }
        thread::sleep(Duration::from_millis(15));
    }
    panic!(
        "timed out waiting for {task_id}: {:?}",
        service.snapshot().unwrap()
    );
}

fn wait_until_idle(service: &Service, task_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if service
            .runs
            .lock()
            .unwrap()
            .tasks
            .get(task_id)
            .is_some_and(|control| control.is_idle())
        {
            return;
        }
        thread::sleep(Duration::from_millis(15));
    }
    panic!("timed out waiting for idle owner: {task_id}");
}

#[test]
fn planned_acp_retirement_wakes_one_saved_native_session_without_reconnect() {
    let fixture = fixture();
    let agent = fixture.service.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .service
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP idle retirement".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let first = fixture
        .service
        .accept_send(task.id.clone(), "first".into(), vec![])
        .unwrap()
        .unwrap();
    fixture
        .service
        .launch_accepted(task.id.clone(), Some(first));
    wait_for(&fixture.service, &task.id, |snapshot| {
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| {
                item.status == "completed"
                    && item.native_session_id.as_deref() == Some("native-retire-id")
            })
    });

    wait_until_idle(&fixture.service, &task.id);
    let before = fixture.service.snapshot().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && fixture.service.collect_idle_runtimes_at(
            Instant::now() + IDLE_RUNTIME_TIMEOUT + Duration::from_millis(1),
        ) == 0
    {
        thread::sleep(Duration::from_millis(15));
    }
    assert_eq!(before, fixture.service.snapshot().unwrap());
    assert!(fixture
        .service
        .runs
        .lock()
        .unwrap()
        .tasks
        .get(&task.id)
        .is_none());

    let second = fixture
        .service
        .accept_send(task.id.clone(), "second".into(), vec![])
        .unwrap()
        .unwrap();
    fixture
        .service
        .launch_accepted(task.id.clone(), Some(second));
    wait_for(&fixture.service, &task.id, |snapshot| {
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| item.status == "completed")
            && snapshot
                .messages
                .iter()
                .filter(|message| message.task_id == task.id && message.role == "assistant")
                .count()
                == 0
    });

    let capture = fs::read_to_string(&fixture.capture).unwrap_or_default();
    assert_eq!(
        capture
            .lines()
            .filter(|line| *line == "session/new")
            .count(),
        1,
        "{capture}"
    );
    assert_eq!(
        capture
            .lines()
            .filter(|line| *line == "session/resume")
            .count(),
        1,
        "{capture}"
    );
    assert_eq!(
        capture
            .lines()
            .filter(|line| *line == "session/prompt")
            .count(),
        2,
        "{capture}"
    );
    assert!(
        !fixture
            .service
            .snapshot()
            .unwrap()
            .events
            .iter()
            .any(|event| {
                event.task_id == task.id
                    && event.title.contains("ACP connection could not be restored")
            }),
        "{capture}"
    );
}
