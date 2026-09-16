//! Local Claude stream-json retirement and cold-resume coverage.

use crate::{model::CreateTaskInput, Service};
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
    log: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!("monitter-claude-idle-{}", crate::id()));
    fs::create_dir_all(&root).unwrap();
    let script = root.join("mock-claude.cjs");
    let log = root.join("claude-launch.log");
    fs::write(&script, r#"#!/usr/bin/env node
const fs=require('node:fs'), readline=require('node:readline');
const log='claude-launch.log';
const args=process.argv.slice(2); fs.appendFileSync(log,`ARGS:${JSON.stringify(args)}\n`);
const session='claude-idle-session'; let turn=0;
const emit=x=>console.log(JSON.stringify(x));
emit({type:'system',subtype:'init',session_id:session});
readline.createInterface({input:process.stdin}).on('line',line=>{
  const f=JSON.parse(line); if(f.type!=='user') return; turn++;
  emit({type:'assistant',session_id:session,message:{content:[{type:'text',text:`reply-${turn}`}]}});
  emit({type:'result',session_id:session,is_error:false,stop_reason:'end_turn',duration_ms:1,num_turns:1});
});
"#).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let service = Service::open(None, root.clone()).unwrap();
    service
        .mutate(None, |s| {
            let a = &mut s.agents[0];
            a.provider = "claude".into();
            a.sandbox = "harness-configured".into();
            a.cwd = root.to_string_lossy().into_owned();
            a.collaboration_enabled = false;
            a.host_id = s.hosts[0].id.clone();
            s.hosts[0].claude_path = script.to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
    Fixture { service, root, log }
}

fn wait_for(f: &Fixture, id: &str, p: impl Fn(&crate::Snapshot) -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        if p(&f.service.snapshot().unwrap()) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out {id}");
}

fn wait_until_idle(f: &Fixture, id: &str) {
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        if f.service
            .runs
            .lock()
            .unwrap()
            .tasks
            .get(id)
            .is_some_and(|control| control.is_idle())
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for idle owner {id}");
}

#[test]
fn claude_idle_retirement_cold_resumes_same_native_session_without_snapshot_mutation() {
    let f = fixture();
    let a = f.service.snapshot().unwrap().agents[0].clone();
    let t = f
        .service
        .create_task(CreateTaskInput {
            agent_id: a.id,
            title: "Claude idle".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    f.service
        .send_fast(t.id.clone(), "first".into(), vec![])
        .unwrap();
    wait_for(&f, &t.id, |s| {
        s.tasks
            .iter()
            .any(|x| x.id == t.id && x.status == "completed")
    });
    wait_until_idle(&f, &t.id);
    let before = f.service.snapshot().unwrap();
    let native = before
        .tasks
        .iter()
        .find(|x| x.id == t.id)
        .unwrap()
        .native_session_id
        .clone()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut retired = 0;
    while Instant::now() < deadline {
        retired = f
            .service
            .collect_idle_runtimes_at(Instant::now() + Duration::from_secs(301));
        if retired == 1 {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(retired, 1);
    assert_eq!(before, f.service.snapshot().unwrap());
    f.service
        .send_fast(t.id.clone(), "second".into(), vec![])
        .unwrap();
    wait_for(&f, &t.id, |s| {
        s.messages
            .iter()
            .filter(|m| m.task_id == t.id && m.role == "assistant")
            .count()
            == 2
    });
    let after = f.service.snapshot().unwrap();
    assert_eq!(
        after
            .tasks
            .iter()
            .find(|x| x.id == t.id)
            .unwrap()
            .native_session_id
            .as_deref(),
        Some(native.as_str())
    );
    let log = fs::read_to_string(&f.log).unwrap();
    let args: Vec<&str> = log.lines().filter(|x| x.starts_with("ARGS:")).collect();
    assert_eq!(args.len(), 2);
    assert!(args[1].contains("--resume") && args[1].contains(&native));
}
