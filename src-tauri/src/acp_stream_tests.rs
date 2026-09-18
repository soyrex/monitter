//! ACP stream identity regression tests.
//!
//! The fixture is a local executable and never contacts a provider.

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
    script: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_file(&self.script);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(mode: Option<&str>) -> Fixture {
    let suffix = crate::id();
    let root = std::env::temp_dir().join(format!("monitter-acp-stream-{suffix}"));
    let script = std::env::temp_dir().join(format!("monitter-acp-stream-{suffix}.mjs"));
    fs::write(&script, r#"#!/usr/bin/env node
import readline from 'node:readline';
const session = 'stream-session';
const reply = (id, result) => console.log(JSON.stringify({jsonrpc:'2.0', id, result}));
const update = (messageId, content) => console.log(JSON.stringify({jsonrpc:'2.0', method:'session/update', params:{sessionId:session, update:{sessionUpdate:'agent_message_chunk', messageId, content}}}));
readline.createInterface({input:process.stdin}).on('line', line => {
  const frame = JSON.parse(line);
  if (frame.method === 'initialize') reply(frame.id, {protocolVersion:1, agentCapabilities:{mcpCapabilities:{http:true}}});
  else if (frame.method === 'session/new') reply(frame.id, {sessionId:session});
  else if (frame.method === 'session/prompt') {
    if (process.argv[2] === 'anonymous') {
      update(undefined, {type:'text', text:'before tool'});
      console.log(JSON.stringify({jsonrpc:'2.0', method:'session/update', params:{sessionId:session, update:{sessionUpdate:'tool_call', toolCallId:'fixture-tool', title:'Read fixture', kind:'read', status:'completed'}}}));
      update(undefined, {type:'text', text:'after tool'});
    } else if (process.argv[2] === 'many-tools') {
      for (let index=0; index<32; index++) {
        console.log(JSON.stringify({jsonrpc:'2.0', method:'session/update', params:{sessionId:session, update:{sessionUpdate:'tool_call', toolCallId:`fixture-tool-${index}`, title:`Read fixture ${index}`, kind:'read', status:'completed', rawOutput:{text:'x'.repeat(70*1024)}}}}));
      }
      update('done', {type:'text', text:'All tools finished.'});
    } else {
      update('a1', {type:'text', text:'a1'});
      update('b1', {type:'text', text:'b1'});
      update('b1', {type:'image', data:'/9j/2Q==', mimeType:'image/jpeg'});
      update('a1', {type:'text', text:'a2'});
    }
    reply(frame.id, {stopReason:'end_turn'});
  }
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
                args: mode.map(str::to_owned).into_iter().collect(),
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

fn wait_for(service: &Service, task_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if service
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.status == "completed")
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for ACP stream fixture");
}

#[test]
fn acp_message_id_chunks_remain_separate_and_preserve_order() {
    let fixture = fixture(None);
    let agent = fixture.service.snapshot().unwrap().agents[0].clone();
    let task = fixture
        .service
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP stream identity".into(),
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
        .service
        .accept_send(task.id.clone(), "group chunks".into(), vec![])
        .unwrap()
        .unwrap();
    fixture
        .service
        .launch_accepted(task.id.clone(), Some(prompt));
    wait_for(&fixture.service, &task.id);
    let messages = fixture
        .service
        .snapshot()
        .unwrap()
        .messages
        .into_iter()
        .filter(|message| {
            message.task_id == task.id
                && message.role == "assistant"
                && message.stream_status.as_deref() == Some("complete")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        messages
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        vec!["a1a2", "b1"]
    );
    assert!(messages[0].attachments.is_empty());
    assert_eq!(messages[1].attachments.len(), 1);
    assert_eq!(messages[1].attachments[0].path, "Generated by ACP agent");
}

#[test]
fn acp_anonymous_chunks_split_at_tool_activity() {
    let fixture = fixture(Some("anonymous"));
    let agent = fixture.service.snapshot().unwrap().agents[0].clone();
    let task = fixture
        .service
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP anonymous stream identity".into(),
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
        .service
        .accept_send(task.id.clone(), "split anonymous chunks".into(), vec![])
        .unwrap()
        .unwrap();
    fixture
        .service
        .launch_accepted(task.id.clone(), Some(prompt));
    wait_for(&fixture.service, &task.id);
    let messages = fixture
        .service
        .snapshot()
        .unwrap()
        .messages
        .into_iter()
        .filter(|message| {
            message.task_id == task.id
                && message.role == "assistant"
                && message.stream_status.as_deref() == Some("complete")
        })
        .map(|message| message.text)
        .collect::<Vec<_>>();
    assert_eq!(messages, vec!["before tool", "after tool"]);
}

#[test]
fn acp_tool_output_over_two_mib_completes_the_turn() {
    let fixture = fixture(Some("many-tools"));
    let agent = fixture.service.snapshot().unwrap().agents[0].clone();
    let task = fixture
        .service
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "ACP long tool turn".into(),
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
        .service
        .accept_send(task.id.clone(), "read many files".into(), vec![])
        .unwrap()
        .unwrap();
    fixture
        .service
        .launch_accepted(task.id.clone(), Some(prompt));
    wait_for(&fixture.service, &task.id);
    let snapshot = fixture.service.snapshot().unwrap();
    assert_eq!(
        snapshot
            .events
            .iter()
            .filter(|event| event.task_id == task.id && event.kind == "tool")
            .count(),
        32
    );
    assert!(snapshot.messages.iter().any(|message| {
        message.task_id == task.id
            && message.role == "assistant"
            && message.text == "All tools finished."
    }));
}
