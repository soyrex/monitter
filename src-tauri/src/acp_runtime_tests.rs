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
let turns=0, session='fixture-session', configAcknowledged=true;
const reply=(id,result)=>console.log(JSON.stringify({jsonrpc:'2.0',id,result}));
const update=(text)=>console.log(JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{sessionId:session,update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text},id:`message-${turns}`}}}));
readline.createInterface({input:process.stdin}).on('line', line=>{
 const frame=JSON.parse(line);
 if(frame.method==='initialize') { const recovery=process.argv[2]==='load'?{loadSession:true}:process.argv[2]==='resume'?{sessionCapabilities:{resume:{}}}:{}; const http=process.argv[2]==='managed-no-http'?{}:{mcpCapabilities:{http:true}}; reply(frame.id,{protocolVersion:1,agentCapabilities:{...recovery,...http}}); }
 else if(frame.method==='session/new') { if(['mcp','mcp-hold-second'].includes(process.argv[2]) && process.argv[3]) { const servers=frame.params.mcpServers ?? []; const headers=(servers[0]?.headers ?? []).map(item=>item.name).join(','); fs.appendFileSync(process.argv[3],`mcp-${servers.length}-${servers[0]?.type ?? ''}-${headers}-${servers[0]?.url ?? ''}\n`); } if(process.argv[2]==='managed' && process.argv[3]) fs.appendFileSync(process.argv[3],`session:${JSON.stringify(frame.params.mcpServers ?? [])}\n`); reply(frame.id,['model','model-delayed'].includes(process.argv[2])?{sessionId:'fixture-session',configOptions:[{id:'opaque-model',name:'Model',category:'model',type:'select',currentValue:'default',options:[{value:'default',name:'Default'},{value:'other/model',name:'Other'}]}]}:{sessionId:'fixture-session'}); }
 else if(frame.method==='session/load'||frame.method==='session/resume'){ session=frame.params.sessionId; reply(frame.id,{}); }
 else if(frame.method==='session/set_config_option'){ if(process.argv[3]) fs.appendFileSync(process.argv[3],`model-${frame.params.configId}-${frame.params.value}\n`); if(process.argv[2]==='model-delayed'){ configAcknowledged=false; setTimeout(()=>{ configAcknowledged=true; if(process.argv[3]) fs.appendFileSync(process.argv[3],'config-ack\n'); reply(frame.id,{}); },180); } else reply(frame.id,{}); }
 else if(frame.method==='session/prompt'){ if(!configAcknowledged && process.argv[3]) fs.appendFileSync(process.argv[3],'prompt-before-config-ack\n'); if(process.argv[2]==='managed' && process.argv[3]) fs.appendFileSync(process.argv[3],`prompt:${frame.params.prompt?.[0]?.text ?? ''}\n`); turns++; if(process.argv[2]==='permission'){ console.log(JSON.stringify({jsonrpc:'2.0',id:'opaque-permission',method:'session/request_permission',params:{sessionId:'fixture-session',toolCall:{title:'Write fixture file',rawInput:{path:'fixture.txt'}},options:[{kind:'allow_once',optionId:'opaque-allow'},{kind:'reject_once',optionId:'opaque-reject'},{kind:'allow_always',optionId:'never-select'}]}})); } else { update(`reply-${turns}`); if(!(process.argv[2]==='mcp-hold-second' && turns===2)) reply(frame.id,{stopReason:'end_turn'}); } }
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
    let deadline = Instant::now() + Duration::from_secs(5);
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
        vec!["managed-no-http".into(), capture.to_string_lossy().into_owned()],
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
