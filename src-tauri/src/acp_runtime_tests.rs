//! End-to-end ACP fixtures. These are temporary local subprocesses only.

use crate::{
    extensions::{ManagedSkill, McpServerConfig, McpTransport},
    model::{AcpLaunch, CreateTaskInput},
    Service,
};
use std::collections::BTreeMap;
use std::{
    fs,
    ops::Deref,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    service: Arc<Service>,
    root: PathBuf,
    script: PathBuf,
}

impl Deref for Fixture {
    type Target = Arc<Service>;
    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_file(&self.script);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture_with_args(name: &str, args: Vec<String>) -> Fixture {
    let root = std::env::temp_dir().join(format!("monitter-acp-runtime-{name}-{}", crate::id()));
    let script = std::env::temp_dir().join(format!("monitter-acp-fixture-{}.mjs", crate::id()));
    fs::write(&script, r#"#!/usr/bin/env node
import readline from 'node:readline'; import fs from 'node:fs';
let turns=0, session='fixture-session', configAcknowledged=true, steerAttempts=0;
const reply=(id,result)=>console.log(JSON.stringify({jsonrpc:'2.0',id,result}));
const reject=(id,code,message)=>console.log(JSON.stringify({jsonrpc:'2.0',id,error:{code,message}}));
const updateItem=(text,id)=>console.log(JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{sessionId:session,update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text},id}}}));
const update=(text)=>updateItem(text,`message-${turns}`);
const usageUpdate=()=>console.log(JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{sessionId:session,update:{sessionUpdate:'usage_update',inputTokens:321,outputTokens:123}}}));
const commandUpdate=()=>console.log(JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{sessionId:session,update:{sessionUpdate:'available_commands_update',availableCommands:[{name:'usage',description:'Show fixture usage',input:{hint:'window'}}]}}}));
const routerTrace=()=>console.log(JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{sessionId:session,update:{sessionUpdate:'router_trace',trace:{traceId:'fixture-route',trigger:'initial_prompt',applied:false,requestedModel:'gpt-6-astra',requestedEffort:'max',newModel:'gpt-5.5',newEffort:'high',applicationError:'model unavailable',confidence:0.9,rationale:'fixture rollback'}}}}));
readline.createInterface({input:process.stdin}).on('line', line=>{
 const frame=JSON.parse(line);
 if(frame.method==='initialize') { const recovery=process.argv[2]==='load'?{loadSession:true}:process.argv[2]==='resume'?{sessionCapabilities:{resume:{}}}:{}; const http=process.argv[2]==='managed-no-http'?{}:{mcpCapabilities:{http:true}}; const steer=process.argv[2]==='mcode-steer'?{'minimax-code/extensions':{version:1,methods:['mcode/session/steer']}}:['mona-steer','mona-steer-retry'].includes(process.argv[2])?{'mona/extensions':{version:1,methods:['mona/session/steer']}}:null; reply(frame.id,{protocolVersion:1,agentCapabilities:{...recovery,...http},...(steer?{_meta:steer}:{})}); }
 else if(frame.method==='session/new') { if(['mcp','mcp-hold-second'].includes(process.argv[2]) && process.argv[3]) { const servers=frame.params.mcpServers ?? []; const headers=(servers[0]?.headers ?? []).map(item=>item.name).join(','); fs.appendFileSync(process.argv[3],`mcp-${servers.length}-${servers[0]?.type ?? ''}-${headers}-${servers[0]?.url ?? ''}\n`); } if(process.argv[2]==='managed' && process.argv[3]) fs.appendFileSync(process.argv[3],`session:${JSON.stringify(frame.params.mcpServers ?? [])}\n`); const mcode=process.argv[2]==='mcode-steer'?{sessionId:'fixture-session',configOptions:[{id:'permissionMode',name:'Permission mode',category:'_permission',type:'select',currentValue:'auto',options:[{value:'default',name:'Ask'},{value:'auto',name:'Auto'},{value:'bypassPermissions',name:'Full access'}]}]}:null; reply(frame.id,mcode??(['model','model-delayed'].includes(process.argv[2])?{sessionId:'fixture-session',configOptions:[{id:'opaque-model',name:'Model',category:'model',type:'select',currentValue:'default',options:[{value:'default',name:'Default'},{value:'other/model',name:'Other'}]}]}:{sessionId:'fixture-session'})); if(process.argv[2]==='commands') commandUpdate(); }
 else if(frame.method==='session/load'||frame.method==='session/resume'){ session=frame.params.sessionId; reply(frame.id,{}); }
 else if(frame.method==='session/set_config_option'){ if(process.argv[3]) fs.appendFileSync(process.argv[3],`model-${frame.params.configId}-${frame.params.value}\n`); if(process.argv[2]==='model-delayed'){ configAcknowledged=false; setTimeout(()=>{ configAcknowledged=true; if(process.argv[3]) fs.appendFileSync(process.argv[3],'config-ack\n'); reply(frame.id,{}); },180); } else reply(frame.id,{}); }
 else if(frame.method==='session/prompt'){ if(!configAcknowledged && process.argv[3]) fs.appendFileSync(process.argv[3],'prompt-before-config-ack\n'); if(['managed','commands'].includes(process.argv[2]) && process.argv[3]) fs.appendFileSync(process.argv[3],`prompt:${frame.params.prompt?.[0]?.text ?? ''}\n`); turns++; if(['mcode-steer','mona-steer','mona-steer-retry'].includes(process.argv[2])){ globalThis.activePromptId=frame.id; if(process.argv[3]) fs.appendFileSync(process.argv[3],`prompt-active:${frame.id}\n`); } else if(['permission','permission-no-allow'].includes(process.argv[2])){ const options=process.argv[2]==='permission-no-allow'?[{kind:'reject_once',optionId:'opaque-reject'}]:[{kind:'allow_once',optionId:'opaque-allow'},{kind:'reject_once',optionId:'opaque-reject'},{kind:'allow_always',optionId:'never-select'}]; console.log(JSON.stringify({jsonrpc:'2.0',id:'opaque-permission',method:'session/request_permission',params:{sessionId:'fixture-session',toolCall:{title:'Write fixture file',rawInput:{path:'fixture.txt'}},options}})); } else { if(process.argv[2]==='router-trace') routerTrace(); if(process.argv[2]==='mona-metadata'){ usageUpdate(); updateItem('first item','metadata-first'); updateItem('final item','metadata-final'); reply(frame.id,{stopReason:'end_turn',model:'MiniMax-M2.7',usage:{inputTokens:321,outputTokens:123},routing:{rationale:'fixture route rationale',requestedModel:'gpt-6-astra',requestedEffort:'xhigh',confidence:0.87,applied:true}}); } else { update(`reply-${turns}`); if(!(process.argv[2]==='mcp-hold-second' && turns===2)) reply(frame.id,{stopReason:'end_turn'}); } } }
 else if(['mcode/session/steer','mona/session/steer'].includes(frame.method)){ steerAttempts++; if(process.argv[3]) fs.appendFileSync(process.argv[3],`steer:${frame.method}:${frame.params.expectedTurnId}:${frame.params.text}\n`); if(process.argv[2]==='mona-steer-retry' && steerAttempts===1){ reject(frame.id,-32001,'prompt is not ready for steering'); } else { reply(frame.id,{turnId:frame.params.expectedTurnId,clientRequestId:frame.params.clientRequestId,mode:'steered'}); update('acp-steered'); reply(globalThis.activePromptId,{stopReason:'end_turn'}); } }
 else if(frame.id==='opaque-permission'){ const outcome=frame.result?.outcome; if(process.argv[3]) fs.appendFileSync(process.argv[3],`outcome-${outcome?.optionId ?? outcome?.outcome}\n`); update(`permission-${outcome?.optionId ?? outcome?.outcome}`); reply(3,{stopReason:'end_turn'}); }
 else if(frame.method==='session/cancel'){ if(process.argv[3]) fs.appendFileSync(process.argv[3],'cancel\n'); }
});
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
            agent.cwd = root.to_string_lossy().into_owned();
            agent.acp = Some(AcpLaunch {
                command: script.to_string_lossy().into_owned(),
                args,
            });
            Ok(())
        })
        .unwrap();
    Fixture {
        service,
        root,
        script,
    }
}

fn fixture(name: &str) -> Fixture {
    fixture_with_args(name, vec![])
}

fn wait_for(service: &Service, task_id: &str, predicate: impl Fn(&crate::Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let snapshot = service.snapshot().unwrap();
        if predicate(&snapshot) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "timed out waiting for ACP fixture {task_id}: {:?}",
        service.snapshot().unwrap()
    );
}

#[test]
fn resident_fixture_delivers_context_and_two_distinct_turns() {
    let fixture = fixture("two-turns");
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP fixture".into(),
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
        .accept_send(task.id.clone(), "first turn".into(), vec![])
        .unwrap()
        .unwrap();
    assert!(first.prompt.contains("You are acting as"));
    fixture.launch_accepted(task.id.clone(), Some(first));
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| item.status == "completed")
    });
    let second = fixture
        .accept_send(task.id.clone(), "second turn".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(second));
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .messages
            .iter()
            .filter(|message| {
                message.task_id == task.id
                    && message.role == "assistant"
                    && message.stream_status.as_deref() == Some("complete")
            })
            .count()
            == 2
    });
    let replies = fixture
        .snapshot()
        .unwrap()
        .messages
        .into_iter()
        .filter(|message| message.task_id == task.id && message.role == "assistant")
        .map(|message| message.text)
        .collect::<Vec<_>>();
    assert_eq!(replies, vec!["reply-1", "reply-2"]);
}

#[test]
fn mona_router_trace_is_visible_as_a_truthful_status_event() {
    let fixture = fixture_with_args("router-trace", vec!["router-trace".into()]);
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "Mona routing fixture".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let accepted = fixture
        .accept_send(task.id.clone(), "route this safely".into(), vec![])
        .unwrap()
        .unwrap();
    // Invoke the accepted-turn dispatcher directly so this subprocess test
    // does not depend on a Tauri async runtime being installed by the test
    // runner. Production still reaches the same dispatcher through
    // `launch_accepted`.
    fixture.deliver_accepted(task.id.clone(), accepted);
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot.events.iter().any(|event| {
            event.task_id == task.id && event.title == "Jev route rolled back · gpt-5.5 · high"
        })
    });

    let event = fixture
        .snapshot()
        .unwrap()
        .events
        .into_iter()
        .find(|event| event.task_id == task.id && event.title.starts_with("Jev route rolled back"))
        .expect("router trace event");
    assert_eq!(event.kind, "status");
    let detail: serde_json::Value = serde_json::from_str(&event.detail).unwrap();
    assert_eq!(detail["trace"]["requestedModel"], "gpt-6-astra");
    assert_eq!(detail["trace"]["newModel"], "gpt-5.5");
    assert_eq!(detail["trace"]["applicationError"], "model unavailable");
}

#[test]
fn mona_prompt_result_persists_metadata_only_on_the_final_assistant_item() {
    let fixture = fixture_with_args("mona-metadata", vec!["mona-metadata".into()]);
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "Mona result metadata fixture".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let accepted = fixture
        .accept_send(task.id.clone(), "report your route".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.deliver_accepted(task.id.clone(), accepted);
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot.messages.iter().any(|message| {
            message.task_id == task.id
                && message.text == "final item"
                && message.response_metadata.is_some()
        })
    });

    let messages = fixture
        .snapshot()
        .unwrap()
        .messages
        .into_iter()
        .filter(|message| message.task_id == task.id && message.role == "assistant")
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].text, "first item");
    assert!(messages[0].response_metadata.is_none());
    assert_eq!(messages[1].text, "final item");
    let metadata = messages[1].response_metadata.as_ref().unwrap();
    assert_eq!(metadata.model, "MiniMax-M2.7");
    assert_eq!(metadata.input_tokens, 321);
    assert_eq!(metadata.output_tokens, 123);
    assert_eq!(
        metadata.jev_rationale.as_deref(),
        Some("fixture route rationale")
    );
    assert_eq!(metadata.requested_model.as_deref(), Some("gpt-6-astra"));
    assert_eq!(metadata.requested_effort.as_deref(), Some("xhigh"));
    assert_eq!(metadata.confidence, Some(0.87));
    assert_eq!(metadata.route_applied, Some(true));
    assert!(!fixture
        .snapshot()
        .unwrap()
        .events
        .iter()
        .any(|event| { event.task_id == task.id && event.title == "Usage capture warning" }));
}

fn run_mona_steer_fixture(mode: &str, expected_attempts: usize) {
    let capture = std::env::temp_dir().join(format!("monitter-{mode}-{}", crate::id()));
    let fixture = fixture_with_args(
        mode,
        vec![mode.into(), capture.to_string_lossy().into_owned()],
    );
    fixture
        .mutate(None, |snapshot| {
            snapshot.settings.busy_message_mode = "steer".into();
            Ok(())
        })
        .unwrap();
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "Mona steer fixture".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let accepted = fixture
        .accept_send(task.id.clone(), "start the long turn".into(), vec![])
        .unwrap()
        .unwrap();
    // Drive the resident transport on an explicit worker. The fixture must
    // remain blocked in session/prompt while this test submits its steer, so
    // relying on Tauri's test-runtime scheduler would introduce a long and
    // unrelated startup race.
    let run_service = Arc::clone(&fixture.service);
    let run_task_id = task.id.clone();
    let run_thread = thread::spawn(move || run_service.deliver_accepted(run_task_id, accepted));

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline
        && !fs::read_to_string(&capture)
            .unwrap_or_default()
            .contains("prompt-active:")
    {
        thread::sleep(Duration::from_millis(20));
    }
    let startup_capture = fs::read_to_string(&capture).unwrap_or_default();
    assert!(
        startup_capture.contains("prompt-active:"),
        "Mona fixture never entered its active ACP prompt. Capture: {startup_capture:?}. Snapshot: {:?}",
        fixture.snapshot().unwrap()
    );

    fixture
        .send_fast(task.id.clone(), "change course safely".into(), vec![])
        .unwrap();
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot.messages.iter().any(|message| {
            message.task_id == task.id
                && message.role == "user"
                && message.text == "change course safely"
        }) && snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| item.status == "completed")
    });

    let snapshot = fixture.snapshot().unwrap();
    assert!(snapshot
        .queued_messages
        .iter()
        .all(|message| message.task_id != task.id));
    assert!(snapshot.events.iter().any(|event| {
        event.task_id == task.id && event.title == "Follow-up steered into active ACP turn"
    }));
    let captured = fs::read_to_string(&capture).unwrap_or_default();
    assert!(
        captured.contains("steer:mona/session/steer:acp:3:change course safely"),
        "Monitter did not send Mona's correlated steering method: {captured}"
    );
    assert_eq!(
        captured
            .lines()
            .filter(|line| line.starts_with("steer:mona/session/steer:"))
            .count(),
        expected_attempts,
        "unexpected Mona steering attempt count: {captured}"
    );
    run_thread.join().unwrap();
    let _ = fs::remove_file(capture);
}

#[test]
fn unavailable_acp_yolo_uses_only_advertised_one_time_permissions() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-yolo-{}", crate::id()));
    let fixture = fixture_with_args(
        "no-bypass-mode",
        vec!["permission".into(), capture.to_string_lossy().into_owned()],
    );
    fixture
        .mutate(None, |snapshot| {
            snapshot.agents[0].sandbox = "yolo".into();
            Ok(())
        })
        .unwrap();
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id.clone(),
            title: "ACP YOLO compatibility".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let accepted = fixture
        .accept_send(
            task.id.clone(),
            "continue with one-time ACP permissions".into(),
            vec![],
        )
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(accepted));
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| item.status == "completed")
    });
    let snapshot = fixture.snapshot().unwrap();
    assert!(snapshot.approval_requests.is_empty());
    assert!(snapshot.events.iter().any(|event| {
        event.task_id == task.id
            && event.title == "ACP YOLO compatibility"
            && event.detail.contains("allow_once")
    }));
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.title == "ACP YOLO permission"));
    assert!(fs::read_to_string(&capture)
        .unwrap_or_default()
        .contains("outcome-opaque-allow"));
    let _ = fs::remove_file(capture);
}

#[test]
fn unavailable_acp_yolo_cancels_a_permission_without_allow_once() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-yolo-cancel-{}", crate::id()));
    let fixture = fixture_with_args(
        "no-allow-once",
        vec![
            "permission-no-allow".into(),
            capture.to_string_lossy().into_owned(),
        ],
    );
    fixture
        .mutate(None, |snapshot| {
            snapshot.agents[0].sandbox = "yolo".into();
            Ok(())
        })
        .unwrap();
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP YOLO missing allow once".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let accepted = fixture
        .accept_send(
            task.id.clone(),
            "do not broaden this permission".into(),
            vec![],
        )
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(accepted));
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| item.status == "completed")
    });
    let snapshot = fixture.snapshot().unwrap();
    assert!(snapshot.approval_requests.is_empty());
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.title == "ACP YOLO permission cancelled"));
    assert!(fs::read_to_string(&capture)
        .unwrap_or_default()
        .contains("outcome-cancelled"));
    let _ = fs::remove_file(capture);
}

#[test]
fn mona_extension_steers_the_exact_active_acp_prompt() {
    run_mona_steer_fixture("mona-steer", 1);
}

#[test]
fn mona_not_ready_steer_is_retried_for_the_same_active_prompt() {
    run_mona_steer_fixture("mona-steer-retry", 2);
}

#[test]
fn resident_fixture_advertises_and_executes_a_raw_slash_command() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-commands-{}", crate::id()));
    let fixture = fixture_with_args(
        "commands",
        vec!["commands".into(), capture.to_string_lossy().into_owned()],
    );
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP command fixture".into(),
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
        .accept_send(task.id.clone(), "establish session".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(first));
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| item.status == "completed")
    });

    let commands = fixture.task_slash_commands(&task.id).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "usage");
    assert_eq!(commands[0].input_hint.as_deref(), Some("window"));
    let result = fixture
        .execute_task_slash_command(task.id.clone(), "/usage week".into())
        .unwrap();
    assert_eq!(result.effect, "sent");
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .messages
            .iter()
            .filter(|message| {
                message.task_id == task.id
                    && message.role == "assistant"
                    && message.stream_status.as_deref() == Some("complete")
            })
            .count()
            == 2
    });
    let captured = fs::read_to_string(&capture).unwrap_or_default();
    assert!(
        captured.lines().any(|line| line == "prompt:/usage week"),
        "provider did not receive the exact slash prompt: {captured}"
    );
    assert!(fixture.snapshot().unwrap().messages.iter().any(|message| {
        message.task_id == task.id && message.role == "user" && message.text == "/usage week"
    }));
    let _ = fs::remove_file(capture);
}

#[test]
fn acp_steering_extension_metadata_is_opt_in_namespaced_and_bounded() {
    let control = crate::runner::RunControl::new(false);
    control.set_acp_extensions(&serde_json::json!({
        "_meta": {"minimax-code/extensions": {
            "version": 1,
            "methods": ["mcode/session/steer"]
        }}
    }));
    assert!(control.supports_acp_steer());
    control.set_acp_extensions(&serde_json::json!({
        "_meta": {"minimax-code/extensions": {"version": 2, "methods": ["mcode/session/steer"]}}
    }));
    assert!(!control.supports_acp_steer());
    control.set_acp_extensions(&serde_json::json!({
        "_meta": {"mona/extensions": {
            "version": 1,
            "methods": ["mona/session/steer"]
        }}
    }));
    assert!(control.supports_acp_steer());
}

#[test]
fn acp_collaboration_uses_scoped_http_server_and_revokes_on_stop() {
    for (enabled, expected) in [
        (true, "mcp-1-http-Authorization-http://127.0.0.1:"),
        (false, "mcp-0---"),
    ] {
        let capture = std::env::temp_dir().join(format!("monitter-acp-mcp-{}", crate::id()));
        let fixture = fixture_with_args(
            "mcp",
            vec!["mcp".into(), capture.to_string_lossy().into_owned()],
        );
        fixture
            .mutate(None, |snapshot| {
                snapshot.agents[0].collaboration_enabled = enabled;
                Ok(())
            })
            .unwrap();
        let agent = fixture.snapshot().unwrap().agents.remove(0);
        let task = fixture
            .create_task(CreateTaskInput {
                agent_id: agent.id,
                title: "ACP MCP fixture".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        let prompt = fixture
            .accept_send(task.id.clone(), "mcp setup".into(), vec![])
            .unwrap()
            .unwrap();
        fixture.launch_accepted(task.id.clone(), Some(prompt));
        wait_for(&fixture, &task.id, |snapshot| {
            snapshot
                .tasks
                .iter()
                .find(|item| item.id == task.id)
                .is_some_and(|item| item.status == "completed")
        });
        let captured = fs::read_to_string(&capture).unwrap_or_default();
        assert!(
            captured.lines().any(|line| line.starts_with(expected)),
            "{captured}"
        );
        assert!(
            !captured.contains("Bearer"),
            "fixture must not record a token: {captured}"
        );

        fixture
            .resident_control(&task.id)
            .unwrap()
            .expect("resident ACP control")
            .terminate_owned();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline
            && fixture
                .collaboration_grants
                .lock()
                .is_ok_and(|grants| grants.contains_key(&task.id))
        {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            fixture
                .collaboration_grants
                .lock()
                .is_ok_and(|grants| !grants.contains_key(&task.id)),
            "scoped collaboration grant must be revoked after stop"
        );
        let _ = fs::remove_file(&capture);
    }
}

#[test]
fn managed_mcp_and_skill_reach_acp_launch_without_rewriting_user_transcript() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-managed-{}", crate::id()));
    let fixture = fixture_with_args(
        "managed",
        vec!["managed".into(), capture.to_string_lossy().into_owned()],
    );
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let mut config = fixture.extension_config().unwrap();
    config.mcp_servers.push(McpServerConfig {
        id: "11111111-1111-4111-8111-111111111111".into(),
        name: "Private docs".into(),
        enabled: true,
        agent_ids: vec![agent.id.clone()],
        transport: McpTransport::Stdio,
        command: "/usr/bin/true".into(),
        args: vec!["private-argument".into()],
        env: BTreeMap::from([("PRIVATE_TOKEN".into(), "fixture-secret".into())]),
        url: String::new(),
        headers: BTreeMap::new(),
    });
    config.skills.push(ManagedSkill {
        id: "22222222-2222-4222-8222-222222222222".into(),
        name: "Boundary check".into(),
        description: String::new(),
        enabled: true,
        agent_ids: vec![agent.id.clone()],
        all_agents: false,
        source_url: None,
        content: "MANAGED_SKILL_SENTINEL".into(),
    });
    fixture.save_extension_config(config).unwrap();

    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "Managed ACP fixture".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let prompt = fixture
        .accept_send(task.id.clone(), "original user request".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(prompt));
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| item.status == "completed")
    });

    let captured = fs::read_to_string(&capture).unwrap();
    let _ = fs::remove_file(&capture);
    assert!(
        captured.contains(r#""name":"11111111-1111-4111-8111-111111111111""#),
        "{captured}"
    );
    assert!(
        captured.contains(r#""name":"PRIVATE_TOKEN","value":"fixture-secret""#),
        "{captured}"
    );
    assert!(
        captured.contains("prompt:Portable skills enabled for this agent:"),
        "{captured}"
    );
    assert!(captured.contains("MANAGED_SKILL_SENTINEL"), "{captured}");

    let stored_user = fixture
        .snapshot()
        .unwrap()
        .messages
        .into_iter()
        .filter(|message| message.task_id == task.id && message.role == "user")
        .map(|message| message.text)
        .collect::<Vec<_>>();
    assert_eq!(stored_user, vec!["original user request"]);
    assert!(!stored_user
        .iter()
        .any(|text| text.contains("MANAGED_SKILL_SENTINEL")));
}

#[test]
fn managed_http_mcp_is_rejected_before_prompt_without_advertised_capability() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-http-{}", crate::id()));
    let fixture = fixture_with_args(
        "managed-http",
        vec![
            "managed-no-http".into(),
            capture.to_string_lossy().into_owned(),
        ],
    );
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let mut config = fixture.extension_config().unwrap();
    config.mcp_servers.push(McpServerConfig {
        id: "33333333-3333-4333-8333-333333333333".into(),
        name: "Remote docs".into(),
        enabled: true,
        agent_ids: vec![agent.id.clone()],
        transport: McpTransport::Http,
        command: String::new(),
        args: vec![],
        env: BTreeMap::new(),
        url: "https://example.test/mcp".into(),
        headers: BTreeMap::from([("Authorization".into(), "fixture-secret".into())]),
    });
    fixture.save_extension_config(config).unwrap();
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "HTTP capability fixture".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let prompt = fixture
        .accept_send(task.id.clone(), "must not prompt".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(prompt));
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .is_some_and(|item| item.status == "error")
    });
    assert!(!fs::read_to_string(&capture)
        .unwrap_or_default()
        .contains("prompt:"));
    let _ = fs::remove_file(&capture);
}

#[test]
fn permission_fixture_returns_exact_one_time_option_for_approve_and_deny() {
    for (decision, expected) in [
        (
            crate::ApprovalDecision::ApproveOnce,
            "permission-opaque-allow",
        ),
        (crate::ApprovalDecision::Deny, "permission-opaque-reject"),
    ] {
        let fixture = fixture_with_args("permission", vec!["permission".into()]);
        let agent = fixture.snapshot().unwrap().agents.remove(0);
        let task = fixture
            .create_task(CreateTaskInput {
                agent_id: agent.id,
                title: "ACP permission".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        let prompt = fixture
            .accept_send(task.id.clone(), "permission turn".into(), vec![])
            .unwrap()
            .unwrap();
        fixture.launch_accepted(task.id.clone(), Some(prompt));
        wait_for(&fixture, &task.id, |snapshot| {
            snapshot
                .approval_requests
                .iter()
                .any(|request| request.task_id == task.id && request.status == "pending")
        });
        let request = fixture
            .snapshot()
            .unwrap()
            .approval_requests
            .into_iter()
            .find(|request| request.task_id == task.id)
            .unwrap();
        assert_eq!(request.tool, "Write fixture file");
        assert!(request.detail.contains("fixture.txt"));
        fixture
            .resolve_approval_request(&request.id, decision)
            .unwrap();
        wait_for(&fixture, &task.id, |snapshot| {
            snapshot.messages.iter().any(|message| {
                message.task_id == task.id
                    && message.text == expected
                    && message.stream_status.as_deref() == Some("complete")
            })
        });
    }
}

#[test]
fn cancelling_pending_permission_emits_cancelled_outcome_then_session_cancel() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-cancel-{}", crate::id()));
    let fixture = fixture_with_args(
        "permission-cancel",
        vec!["permission".into(), capture.to_string_lossy().into_owned()],
    );
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP cancel".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let prompt = fixture
        .accept_send(task.id.clone(), "cancel permission".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(prompt));
    wait_for(&fixture, &task.id, |snapshot| {
        snapshot
            .approval_requests
            .iter()
            .any(|request| request.task_id == task.id && request.status == "pending")
    });
    fixture.cancel(&task.id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && !capture.exists() {
        thread::sleep(Duration::from_millis(20));
    }
    let captured = fs::read_to_string(&capture).unwrap_or_default();
    let _ = fs::remove_file(&capture);
    assert!(captured.contains("outcome-cancelled"), "{captured}");
    assert!(captured.contains("cancel"), "{captured}");
}

#[test]
fn load_and_resume_keep_saved_native_session_when_response_omits_id() {
    for mode in ["load", "resume"] {
        let fixture = fixture_with_args("recovery", vec![mode.into()]);
        let agent = fixture.snapshot().unwrap().agents.remove(0);
        let task = fixture
            .create_task(CreateTaskInput {
                agent_id: agent.id,
                title: "ACP recovery".into(),
                native_session_id: Some("saved-session".into()),
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        let prompt = fixture
            .accept_send(task.id.clone(), "recover".into(), vec![])
            .unwrap()
            .unwrap();
        fixture.launch_accepted(task.id.clone(), Some(prompt));
        wait_for(&fixture, &task.id, |snapshot| {
            snapshot
                .tasks
                .iter()
                .find(|item| item.id == task.id)
                .is_some_and(|item| item.status == "completed")
        });
        assert_eq!(
            fixture
                .snapshot()
                .unwrap()
                .tasks
                .into_iter()
                .find(|item| item.id == task.id)
                .unwrap()
                .native_session_id
                .as_deref(),
            Some("saved-session")
        );
    }
}

#[test]
fn resident_model_acknowledgement_precedes_each_prompt() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-model-{}", crate::id()));
    let fixture = fixture_with_args(
        "model",
        vec![
            "model-delayed".into(),
            capture.to_string_lossy().into_owned(),
        ],
    );
    fixture
        .mutate(None, |snapshot| {
            snapshot.agents[0].model = "other/model".into();
            Ok(())
        })
        .unwrap();
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP model".into(),
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
        .accept_send(task.id.clone(), "first".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(first));
    wait_for(&fixture, &task.id, |s| {
        s.tasks
            .iter()
            .find(|t| t.id == task.id)
            .is_some_and(|t| t.status == "completed")
    });
    let second = fixture
        .accept_send(task.id.clone(), "second".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(second));
    wait_for(&fixture, &task.id, |s| {
        s.messages
            .iter()
            .filter(|m| m.task_id == task.id && m.role == "assistant")
            .count()
            == 2
    });
    let captured = fs::read_to_string(&capture).unwrap_or_default();
    let _ = fs::remove_file(&capture);
    assert_eq!(
        captured
            .lines()
            .filter(|line| *line == "model-opaque-model-other/model")
            .count(),
        2,
        "{captured}"
    );
    assert!(
        !captured.contains("prompt-before-config-ack"),
        "prompt escaped the model acknowledgement barrier: {captured}"
    );
    assert_eq!(
        captured
            .lines()
            .filter(|line| *line == "config-ack")
            .count(),
        2,
        "{captured}"
    );
}

#[test]
fn resident_acp_rejects_a_concurrent_turn_while_model_configuration_is_pending() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-concurrent-{}", crate::id()));
    let fixture = fixture_with_args(
        "concurrent",
        vec![
            "model-delayed".into(),
            capture.to_string_lossy().into_owned(),
        ],
    );
    fixture
        .mutate(None, |snapshot| {
            snapshot.agents[0].model = "other/model".into();
            Ok(())
        })
        .unwrap();
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP concurrent".into(),
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
        .accept_send(task.id.clone(), "first".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(first));
    wait_for(&fixture, &task.id, |s| {
        s.tasks
            .iter()
            .find(|t| t.id == task.id)
            .is_some_and(|t| t.status == "completed")
    });

    let control = fixture
        .runs
        .lock()
        .unwrap()
        .tasks
        .get(&task.id)
        .cloned()
        .expect("resident ACP control");
    control.send_acp_turn("reserved", &task).unwrap();
    let error = control.send_acp_turn("must-not-send", &task).unwrap_err();
    assert!(error.contains("already pending"), "{error}");

    thread::sleep(Duration::from_millis(260));
    let captured = fs::read_to_string(&capture).unwrap_or_default();
    let _ = fs::remove_file(&capture);
    assert!(
        !captured.contains("prompt-before-config-ack"),
        "prompt escaped the config barrier: {captured}"
    );
    assert_eq!(
        captured
            .lines()
            .filter(|line| *line == "model-opaque-model-other/model")
            .count(),
        2,
        "only the initial and reserved sends may configure a model: {captured}"
    );
}

#[test]
fn ssh_shim_runs_real_supervisor_with_spaced_cwd_two_turns_and_cancel() {
    let capture = std::env::temp_dir().join(format!("monitter-acp-ssh-mcp-{}", crate::id()));
    let fixture = fixture_with_args(
        "ssh-supervisor",
        vec![
            "mcp-hold-second".into(),
            capture.to_string_lossy().into_owned(),
        ],
    );
    let shim = fixture.root.join("ssh shim");
    fs::write(
        &shim,
        "#!/bin/sh\nfor item do\n  if [ \"$item\" = \"-N\" ]; then\n    echo 'Allocated port 45123 for remote forward to 127.0.0.1 port 1' >&2\n    while :; do sleep 1; done\n  fi\n  last=$item\ndone\nexec /bin/sh -c \"$last\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let cwd = fixture.root.join("remote cwd with spaces");
    fs::create_dir_all(&cwd).unwrap();
    fixture
        .mutate(None, |snapshot| {
            let host = &mut snapshot.hosts[0];
            host.kind = "ssh".into();
            host.address = "fixture".into();
            host.default_cwd = cwd.to_string_lossy().into_owned();
            let agent = &mut snapshot.agents[0];
            agent.cwd = host.default_cwd.clone();
            Ok(())
        })
        .unwrap();
    let host_id = fixture.snapshot().unwrap().hosts[0].id.clone();
    let _ssh = crate::runner::override_ssh_for_test(&host_id, &shim);
    let agent = fixture.snapshot().unwrap().agents.remove(0);
    let task = fixture
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "SSH ACP".into(),
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
        .accept_send(task.id.clone(), "one".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(first));
    wait_for(&fixture, &task.id, |s| {
        s.tasks
            .iter()
            .find(|t| t.id == task.id)
            .is_some_and(|t| t.status == "completed")
    });
    let second = fixture
        .accept_send(task.id.clone(), "two".into(), vec![])
        .unwrap()
        .unwrap();
    fixture.launch_accepted(task.id.clone(), Some(second));
    wait_for(&fixture, &task.id, |s| {
        s.messages
            .iter()
            .filter(|m| m.task_id == task.id && m.role == "assistant")
            .count()
            == 2
    });
    assert_eq!(
        fixture
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .map(|item| item.status.as_str()),
        Some("running"),
        "the second ACP prompt must remain active until Stop"
    );
    let captured = fs::read_to_string(&capture).unwrap_or_default();
    assert!(
        captured.contains("mcp-1-http-Authorization-http://127.0.0.1:45123/mcp"),
        "SSH ACP must receive the reverse-tunnel endpoint, not the desktop broker endpoint: {captured}"
    );
    fixture.cancel(&task.id).unwrap();
    wait_for(&fixture, &task.id, |s| {
        s.tasks
            .iter()
            .find(|t| t.id == task.id)
            .is_some_and(|t| t.status == "interrupted")
    });
    let cancel_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < cancel_deadline
        && !fs::read_to_string(&capture)
            .unwrap_or_default()
            .contains("cancel")
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        fs::read_to_string(&capture)
            .unwrap_or_default()
            .contains("cancel"),
        "Stop must cancel the active ACP session"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline
        && fixture
            .collaboration_grants
            .lock()
            .is_ok_and(|grants| grants.contains_key(&task.id))
    {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(fixture
        .collaboration_grants
        .lock()
        .is_ok_and(|grants| !grants.contains_key(&task.id)));
    let _ = fs::remove_file(&capture);
    let _ = fs::remove_file(&shim);
}
