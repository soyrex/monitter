//! Explicit Resume after an ACP pipe failure.
//!
//! The fixture is a local JSON-RPC subprocess only; it never contacts a model
//! or network. The first process drops the pipe after `session/new`, while the
//! replacement accepts the saved session and the explicit continuation.

use crate::{
    model::{AcpLaunch, CreateTaskInput},
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
    log: PathBuf,
    starts: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(mode: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("monitter-acp-resume-{}", crate::id()));
    fs::create_dir_all(&root).unwrap();
    let script = root.join("fixture.cjs");
    let log = root.join("frames.log");
    let starts = root.join("starts");
    fs::write(
        &script,
        r#"#!/usr/bin/env node
const fs=require('node:fs'), readline=require('node:readline');
const mode=process.argv[2], log=process.argv[3], starts=process.argv[4];
fs.appendFileSync(starts,'1'); const generation=fs.readFileSync(starts,'utf8').length;
const note=s=>fs.appendFileSync(log,s+'\n');
const out=(id,result)=>{note('response:'+id); console.log(JSON.stringify({jsonrpc:'2.0',id,result}));};
const err=(id,message)=>{note('error:'+message); console.log(JSON.stringify({jsonrpc:'2.0',id,error:{code:-32001,message}}));};
const update=text=>console.log(JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{sessionId:'resume-session',update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text},id:'reply'}}}));
readline.createInterface({input:process.stdin}).on('line', line=>{
  const f=JSON.parse(line); if(f.method) note(f.method); if(f.method==='session/prompt') note('prompt:'+f.params.prompt?.[0]?.text);
  if(f.method==='initialize') out(f.id,{protocolVersion:1,agentCapabilities:{sessionCapabilities:{resume:{}}}});
  else if(f.method==='session/new') out(f.id,{sessionId:'resume-session'});
  else if(f.method==='session/resume') out(f.id,{});
  else if(f.method==='session/prompt' && generation===1 && mode==='error') { err(f.id,'fixture prompt failed'); setTimeout(()=>process.exit(0),10); }
  else if(f.method==='session/prompt' && generation===1) process.exit(0);
  else if(f.method==='session/prompt') { update('continuation-reply'); out(f.id,{stopReason:'end_turn'}); }
});
"#,
    )
    .unwrap();
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
                args: vec![
                    mode.into(),
                    log.to_string_lossy().into_owned(),
                    starts.to_string_lossy().into_owned(),
                ],
            });
            Ok(())
        })
        .unwrap();
    Fixture {
        service,
        root,
        log,
        starts,
    }
}

fn wait_for(f: &Fixture, task_id: &str, predicate: impl Fn(&crate::Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if predicate(&f.service.snapshot().unwrap()) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "timed out waiting for {task_id}: {:?}",
        f.service.snapshot().unwrap()
    );
}

fn exercise_resume_after_failure(mode: &str, first_prompt: &str) {
    let f = fixture(mode);
    let agent = f.service.snapshot().unwrap().agents[0].clone();
    let task = f
        .service
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP Resume failure".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let first = f
        .service
        .accept_send(task.id.clone(), first_prompt.into(), vec![])
        .unwrap()
        .unwrap();
    f.service.launch_accepted(task.id.clone(), Some(first));
    wait_for(&f, &task.id, |s| {
        s.tasks
            .iter()
            .any(|t| t.id == task.id && matches!(t.status.as_str(), "error" | "interrupted"))
            && fs::read_to_string(&f.starts)
                .map(|v| v.len() >= 1)
                .unwrap_or(false)
    });
    let saved = f
        .service
        .snapshot()
        .unwrap()
        .tasks
        .into_iter()
        .find(|t| t.id == task.id)
        .unwrap();
    assert_eq!(saved.native_session_id.as_deref(), Some("resume-session"));

    f.service.resume(task.id.clone()).unwrap();
    wait_for(&f, &task.id, |s| {
        s.tasks
            .iter()
            .any(|t| t.id == task.id && t.status == "completed")
    });
    let log = fs::read_to_string(&f.log).unwrap_or_default();
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("session/new"))
            .count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("session/resume"))
            .count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("prompt:"))
            .count(),
        2,
        "{log}"
    );
    assert!(log.lines().any(|line| line.contains(first_prompt)));
    assert!(log
        .lines()
        .any(|line| line.contains("Continue from where we left off")));
}

#[test]
fn explicit_resume_after_failed_idle_acp_turn_uses_saved_session_once() {
    exercise_resume_after_failure("pipe", "ambiguous first prompt");
}

#[test]
fn explicit_resume_after_acp_json_rpc_error_uses_saved_session_once() {
    exercise_resume_after_failure("error", "failed first prompt");
}
