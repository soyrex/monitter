//! ACP automatic-recovery regression tests.
//!
//! These tests deliberately use a tiny local JSON-RPC subprocess.  The
//! subprocess is not an agent and never makes a model or network request; its
//! only purpose is to make process loss and reconnect ordering observable.

#![allow(dead_code)]

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

struct RecoveryFixture {
    service: Arc<Service>,
    root: PathBuf,
    script: PathBuf,
    log: PathBuf,
    count: PathBuf,
}

impl Drop for RecoveryFixture {
    fn drop(&mut self) {
        self.service.cleanup();
        // KEEP_FIXTURE_DIR=1 lets a developer preserve the temp fixture
        // directory across a failing test run to inspect
        // frames.log / starts / the fake-acp.mjs script.
        if std::env::var_os("KEEP_FIXTURE_DIR").is_some() {
            eprintln!("[recovery-fixture] preserved at {}", self.root.display());
        } else {
            let _ = fs::remove_file(&self.script);
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

/// argv[2] selects the failure point.  Every input frame is recorded as
/// `method` (or `response:<id>`), allowing tests to prove that an ambiguous
/// prompt was not sent a second time.  The count file makes hot-loop bounds
/// independent of process IDs and platform-specific diagnostics.
fn fixture(mode: &str) -> RecoveryFixture {
    let root = std::env::temp_dir().join(format!("monitter-acp-recovery-{}", crate::id()));
    let script = root.join("fake-acp.mjs");
    let log = root.join("frames.log");
    let count = root.join("starts");
    fs::create_dir_all(&root).unwrap();
    fs::write(&script, r#"#!/usr/bin/env node
import readline from 'node:readline'; import fs from 'node:fs';
const mode=process.argv[2], log=process.argv[3], count=process.argv[4];
fs.appendFileSync(count,'1');
const starts=()=>fs.readFileSync(count,'utf8').length;
let session='fixture-session'; let prompted=false;
const write=x=>fs.appendFileSync(log,x+'\n');
const reply=(id,result)=>{ write('response:'+id); console.log(JSON.stringify({jsonrpc:'2.0',id,result})); };
const update=t=>{ write('update:'+t); console.log(JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{sessionId:session,update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:t},id:'fixture-message'}}})); };
readline.createInterface({input:process.stdin}).on('line',line=>{
  const f=JSON.parse(line); if(f.method) write(f.method);
  if(f.method==='initialize') {
    const recovery=mode==='unsupported-load'?{}:mode==='load-history'?{loadSession:true}:{sessionCapabilities:{load:{},resume:{}}};
    const caps={...recovery,mcpCapabilities:{http:true}};
    reply(f.id,{protocolVersion:1,agentCapabilities:caps});
  } else if(f.method==='session/new') {
    if(mode==='known-unsent-failure') console.log(JSON.stringify({jsonrpc:'2.0',id:f.id,error:{code:-32001,message:'fixture session setup failed'}}));
    else reply(f.id,{sessionId:session});
  }
  else if(f.method==='session/load'||f.method==='session/resume') {
    session=f.params.sessionId;
    if(mode==='load-history') update('historical-context-from-load');
    if(mode==='delay-recovery') setTimeout(()=>reply(f.id,{}),250);
    else reply(f.id,{});
    if(mode==='recovery-handshake-eof' && starts()>=2) setTimeout(()=>process.exit(0),10);
  }
  else if(f.method==='session/prompt') {
    prompted=true; write('prompt:'+f.params.prompt?.[0]?.text);
    if((mode==='recovery-handshake-eof' || mode==='delay-recovery' || mode==='double-loss') && starts()===1) process.exit(0);
    if(mode==='double-loss' && starts()===2) { update('second-reply'); reply(f.id,{stopReason:'end_turn'}); setTimeout(()=>process.exit(0),20); return; }
    if(mode==='active-eof') { update('partial-before-eof'); process.exit(0); }
    if(mode==='idle-eof') { update('first-reply'); reply(f.id,{stopReason:'end_turn'}); setTimeout(()=>process.exit(0),20); }
    else if(mode==='always-eof') process.exit(0);
    else { update('reply'); reply(f.id,{stopReason:'end_turn'}); if(mode==='load-history') setTimeout(()=>process.exit(0),20); }
  } else if(f.method==='session/cancel') { write('cancel'); process.exit(0); }
});
if(mode==='always-eof') setTimeout(()=>process.exit(0),10);
"#).unwrap();
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
            agent.cwd = root.to_string_lossy().into();
            agent.acp = Some(AcpLaunch {
                command: script.to_string_lossy().into(),
                args: vec![
                    mode.into(),
                    log.to_string_lossy().into(),
                    count.to_string_lossy().into(),
                ],
            });
            Ok(())
        })
        .unwrap();
    RecoveryFixture {
        service,
        root,
        script,
        log,
        count,
    }
}

fn task(f: &RecoveryFixture, native: Option<&str>) -> crate::model::Task {
    let agent = f.service.snapshot().unwrap().agents[0].clone();
    f.service
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP recovery fixture".into(),
            native_session_id: native.map(str::to_owned),
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap()
}

fn wait_for(f: &RecoveryFixture, id: &str, p: impl Fn(&crate::Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if p(&f.service.snapshot().unwrap()) {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!(
        "timed out waiting for {id}: {:?}",
        f.service.snapshot().unwrap()
    );
}

fn send(f: &RecoveryFixture, t: &crate::model::Task, text: &str) {
    let p = f
        .service
        .accept_send(t.id.clone(), text.into(), vec![])
        .unwrap()
        .unwrap();
    f.service.launch_accepted(t.id.clone(), Some(p));
}

#[test]
fn idle_eof_reconnects_saved_session_without_error_or_prompt_replay() {
    let f = fixture("idle-eof");
    let t = task(&f, None);
    send(&f, &t, "first");
    wait_for(&f, &t.id, |s| {
        s.tasks
            .iter()
            .any(|x| x.id == t.id && x.status == "completed")
    });
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.count)
            .map(|x| x.len() >= 2)
            .unwrap_or(false)
    });
    f.service
        .send_accepted(t.id.clone(), "second".into(), vec![])
        .unwrap();
    wait_for(&f, &t.id, |s| {
        s.messages
            .iter()
            .filter(|m| m.task_id == t.id && m.role == "assistant")
            .count()
            == 2
    });
    let s = f.service.snapshot().unwrap();
    assert!(!s
        .events
        .iter()
        .any(|e| e.task_id == t.id && e.kind == "error"));
    let prompts = fs::read_to_string(&f.log)
        .unwrap_or_default()
        .lines()
        .filter(|x| x.starts_with("prompt:"))
        .count();
    assert_eq!(prompts, 2);
}

#[test]
fn active_eof_marks_turn_interrupted_and_never_replays_prompt() {
    let f = fixture("active-eof");
    let t = task(&f, None);
    send(&f, &t, "ambiguous");
    wait_for(&f, &t.id, |s| {
        s.tasks
            .iter()
            .any(|x| x.id == t.id && x.status == "interrupted")
    });
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.count)
            .map(|x| x.len() >= 2)
            .unwrap_or(false)
    });
    let log = fs::read_to_string(&f.log).unwrap_or_default();
    assert_eq!(log.lines().filter(|x| x.starts_with("prompt:")).count(), 1);
    assert!(f
        .service
        .snapshot()
        .unwrap()
        .messages
        .iter()
        .any(|m| m.task_id == t.id && m.role == "user" && m.text.contains("ambiguous")));
}

#[test]
fn unsupported_saved_session_keeps_history_and_reports_capability_error() {
    let f = fixture("unsupported-load");
    let t = task(&f, Some("saved-session"));
    send(&f, &t, "recover");
    wait_for(&f, &t.id, |s| {
        s.tasks.iter().any(|x| x.id == t.id && x.status == "error")
    });
    let s = f.service.snapshot().unwrap();
    let t = s.tasks.iter().find(|x| x.id == t.id).unwrap();
    assert_eq!(t.native_session_id.as_deref(), Some("saved-session"));
    assert!(s
        .messages
        .iter()
        .any(|m| m.task_id == t.id && m.role == "user" && m.text.contains("recover")));
}

#[test]
fn repeated_process_loss_is_bounded_and_cancellation_stops_recovery() {
    let f = fixture("recovery-handshake-eof");
    let t = task(&f, None);
    send(&f, &t, "stop eventually");
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.count)
            .map(|x| x.len() >= 2)
            .unwrap_or(false)
    });
    wait_for(&f, &t.id, |s| {
        s.tasks
            .iter()
            .any(|x| x.id == t.id && (x.status == "error" || x.status == "interrupted"))
    });
    thread::sleep(Duration::from_millis(500));
    let after = fs::read_to_string(&f.count).unwrap_or_default().len();
    assert!(
        after <= 2,
        "recovery hot loop after saved session: {after} starts"
    );
}

#[test]
fn cancellation_during_delayed_recovery_handshake_prevents_respawn() {
    let f = fixture("delay-recovery");
    let t = task(&f, None);
    send(&f, &t, "cancel during recovery");
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.count)
            .map(|x| x.len() >= 2)
            .unwrap_or(false)
    });
    f.service
        .resident_control(&t.id)
        .unwrap()
        .expect("recovering ACP control")
        .terminate_owned();
    wait_for(&f, &t.id, |s| {
        s.tasks
            .iter()
            .any(|x| x.id == t.id && x.status == "interrupted")
    });
    thread::sleep(Duration::from_millis(500));
    let starts = fs::read_to_string(&f.count).unwrap_or_default().len();
    assert_eq!(
        starts, 2,
        "cancellation during recovery respawned ACP process"
    );
}

#[test]
fn explicit_send_during_recovery_handshake_is_preserved_and_completes() {
    let f = fixture("delay-recovery");
    let t = task(&f, None);
    send(&f, &t, "first ambiguous turn");
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.count)
            .map(|x| x.len() >= 2)
            .unwrap_or(false)
    });
    f.service
        .send_accepted(t.id.clone(), "during recovery".into(), vec![])
        .unwrap();
    wait_for(&f, &t.id, |s| {
        s.messages
            .iter()
            .any(|m| m.task_id == t.id && m.role == "assistant" && m.text.contains("reply"))
    });
    assert!(f
        .service
        .snapshot()
        .unwrap()
        .messages
        .iter()
        .any(|m| m.task_id == t.id && m.role == "user" && m.text.contains("during recovery")));
}

#[test]
fn successful_new_turn_resets_recovery_budget_for_a_second_process_loss() {
    let f = fixture("double-loss");
    let t = task(&f, None);
    send(&f, &t, "first ambiguous turn");
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.count)
            .map(|x| x.len() >= 2)
            .unwrap_or(false)
    });
    f.service
        .send_accepted(t.id.clone(), "new turn after recovery".into(), vec![])
        .unwrap();
    wait_for(&f, &t.id, |s| {
        s.messages
            .iter()
            .filter(|m| m.task_id == t.id && m.role == "assistant")
            .count()
            == 1
    });
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.count)
            .map(|x| x.len() >= 3)
            .unwrap_or(false)
    });
    f.service
        .resident_control(&t.id)
        .unwrap()
        .expect("resident ACP control after second loss")
        .terminate_owned();
    let starts = fs::read_to_string(&f.count).unwrap_or_default().len();
    assert_eq!(
        starts, 3,
        "second loss did not get an independent reconnect budget"
    );
}

#[test]
fn known_unsent_session_failure_does_not_create_or_replay_a_prompt() {
    let f = fixture("known-unsent-failure");
    let t = task(&f, None);
    send(&f, &t, "not sent");
    wait_for(&f, &t.id, |s| {
        s.tasks.iter().any(|x| x.id == t.id && x.status == "error")
    });
    let log = fs::read_to_string(&f.log).unwrap_or_default();
    assert!(
        !log.lines().any(|x| x.starts_with("prompt:")),
        "prompt was sent after known setup failure: {log}"
    );
    assert!(f
        .service
        .snapshot()
        .unwrap()
        .messages
        .iter()
        .any(|m| m.task_id == t.id && m.role == "user" && m.text.contains("not sent")));
}

#[test]
fn queued_followup_after_ambiguous_eof_is_not_replayed() {
    let f = fixture("active-eof");
    let t = task(&f, None);
    let first = f
        .service
        .accept_send(t.id.clone(), "ambiguous first".into(), vec![])
        .unwrap()
        .unwrap();
    f.service.launch_accepted(t.id.clone(), Some(first));
    let queued = f
        .service
        .accept_send(t.id.clone(), "queued followup".into(), vec![])
        .unwrap();
    assert!(queued.is_none());
    wait_for(&f, &t.id, |s| {
        s.tasks
            .iter()
            .any(|x| x.id == t.id && x.status == "interrupted")
    });
    let s = f.service.snapshot().unwrap();
    assert!(s
        .queued_messages
        .iter()
        .any(|m| m.task_id == t.id && m.text == "queued followup"));
    let log = fs::read_to_string(&f.log).unwrap_or_default();
    assert_eq!(log.lines().filter(|x| x.starts_with("prompt:")).count(), 1);
}

#[test]
fn load_history_updates_are_not_mirrored_as_new_monitter_messages() {
    let f = fixture("load-history");
    let t = task(&f, None);
    send(&f, &t, "first");
    wait_for(&f, &t.id, |s| {
        s.tasks
            .iter()
            .any(|x| x.id == t.id && x.status == "completed")
    });
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.count)
            .map(|x| x.len() >= 2)
            .unwrap_or(false)
    });
    wait_for(&f, &t.id, |_| {
        fs::read_to_string(&f.log)
            .map(|x| x.contains("update:historical-context-from-load"))
            .unwrap_or(false)
            && f.service
                .resident_control(&t.id)
                .unwrap()
                .is_some_and(|control| control.current_app_server_thread().is_some())
    });
    let s = f.service.snapshot().unwrap();
    assert_eq!(
        s.messages
            .iter()
            .filter(|m| m.task_id == t.id && m.role == "assistant")
            .count(),
        1
    );
    assert!(!s
        .messages
        .iter()
        .any(|m| m.text.contains("historical-context-from-load")));
    assert!(!s
        .events
        .iter()
        .any(|e| e.task_id == t.id && e.kind == "tool" && e.detail.contains("historical")));
}
