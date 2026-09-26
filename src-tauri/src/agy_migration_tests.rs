use crate::{
    model::{self, AcpLaunch, CreateTaskInput},
    upgrade_local_gemini_agent_profiles, Service,
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[test]
fn retired_local_gemini_preset_upgrades_without_retargeting_existing_chat() {
    let mut snapshot = model::default_snapshot();
    let mut agent = snapshot.agents[0].clone();
    agent.provider = "acp".into();
    agent.model = "gemini-3.5-flash".into();
    agent.collaboration_enabled = true;
    agent.jev_decisions_enabled = true;
    agent.acp = Some(AcpLaunch {
        command: "/old/bin/gemini".into(),
        args: vec!["--acp".into()],
    });
    let task = model::task_from_agent(
        &agent,
        &CreateTaskInput {
            agent_id: agent.id.clone(),
            title: "Old Gemini chat".into(),
            native_session_id: Some("old-gemini-session".into()),
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        },
    );
    snapshot.agents[0] = agent;
    snapshot.tasks.push(task.clone());

    assert!(upgrade_local_gemini_agent_profiles(&mut snapshot));
    assert_eq!(
        snapshot.agents[0].acp.as_ref().unwrap().command,
        "monitter-agy-acp"
    );
    assert!(snapshot.agents[0].acp.as_ref().unwrap().args.is_empty());
    assert!(snapshot.agents[0].model.is_empty());
    assert!(!snapshot.agents[0].collaboration_enabled);
    assert!(!snapshot.agents[0].jev_decisions_enabled);
    assert_eq!(snapshot.tasks[0], task);
    assert!(!upgrade_local_gemini_agent_profiles(&mut snapshot));
}

#[test]
fn custom_and_remote_gemini_launches_are_preserved() {
    let mut snapshot = model::default_snapshot();
    let mut custom = snapshot.agents[0].clone();
    custom.provider = "acp".into();
    custom.acp = Some(AcpLaunch {
        command: "gemini".into(),
        args: vec!["--acp".into(), "--debug".into()],
    });
    let mut remote = custom.clone();
    remote.id = model::id();
    let mut remote_host = snapshot.hosts[0].clone();
    remote_host.id = model::id();
    remote_host.kind = "ssh".into();
    remote.host_id = remote_host.id.clone();
    remote.acp.as_mut().unwrap().args = vec!["--acp".into()];
    snapshot.hosts.push(remote_host);
    snapshot.agents = vec![custom, remote];
    let original = snapshot.agents.clone();

    assert!(!upgrade_local_gemini_agent_profiles(&mut snapshot));
    assert_eq!(snapshot.agents, original);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "Requires an already authenticated local AGY CLI and makes live model turns"]
fn live_agy_bridge_runs_chat_and_file_tool() {
    let root = std::env::temp_dir().join(format!("monitter-live-agy-{}", model::id()));
    fs::create_dir_all(&root).unwrap();
    let service = Service::open(None, root.join("state")).unwrap();
    service.mutate(None, |snapshot| {
        let agent = &mut snapshot.agents[0];
        agent.provider = "acp".into();
        agent.cwd = root.to_string_lossy().into_owned();
        agent.model.clear();
        agent.sandbox = "harness-configured".into();
        agent.collaboration_enabled = false;
        agent.jev_decisions_enabled = false;
        agent.instructions = "For this transport smoke, follow exact reply formats. Use tools only when requested.".into();
        agent.acp = Some(AcpLaunch { command: "monitter-agy-acp".into(), args: vec![] });
        Ok(())
    }).unwrap();
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let task = service
        .create_task(CreateTaskInput {
            agent_id,
            title: "AGY transport smoke".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: Some(root.to_string_lossy().into_owned()),
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let word = format!("plum-{}", model::id());
    service
        .send_fast(
            task.id.clone(),
            format!("Remember this word: {word}. Reply exactly READY. Do not use tools."),
            vec![],
        )
        .unwrap();
    let first = wait_for_agy_turn(&service, &task.id, 1);
    assert!(first
        .messages
        .iter()
        .any(|message| message.task_id == task.id
            && message.role == "assistant"
            && message.text.trim() == "READY"));
    let session_id = first
        .tasks
        .iter()
        .find(|item| item.id == task.id)
        .unwrap()
        .native_session_id
        .clone()
        .unwrap();

    service
        .send_fast(
            task.id.clone(),
            "What word did I ask you to remember? Reply with only that word. Do not use tools."
                .into(),
            vec![],
        )
        .unwrap();
    let second = wait_for_agy_turn(&service, &task.id, 2);
    assert!(second
        .messages
        .iter()
        .any(|message| message.task_id == task.id
            && message.role == "assistant"
            && message.text.trim() == word));
    assert_eq!(
        second
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .unwrap()
            .native_session_id
            .as_deref(),
        Some(session_id.as_str())
    );

    service.send_fast(task.id.clone(), "Use your write_to_file tool to create agy-smoke.txt in the current workspace with exactly the text AGY_TOOL_OK. Then reply DONE.".into(), vec![]).unwrap();
    wait_for_agy_turn(&service, &task.id, 3);
    assert_eq!(
        fs::read_to_string(root.join("agy-smoke.txt"))
            .unwrap()
            .trim(),
        "AGY_TOOL_OK"
    );
    service.cleanup();
    let _ = fs::remove_dir_all(root);
}

fn wait_for_agy_turn(service: &Service, task_id: &str, replies: usize) -> crate::Snapshot {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let snapshot = service.snapshot().unwrap();
        let task = snapshot
            .tasks
            .iter()
            .find(|item| item.id == task_id)
            .unwrap();
        let complete = snapshot
            .messages
            .iter()
            .filter(|message| {
                message.task_id == task_id
                    && message.role == "assistant"
                    && message.stream_status.as_deref() == Some("complete")
            })
            .count();
        if task.status == "completed" && complete >= replies {
            return snapshot;
        }
        assert_eq!(
            task.status,
            "running",
            "AGY task failed: {:?}",
            snapshot
                .events
                .iter()
                .filter(|event| event.kind == "error")
                .collect::<Vec<_>>()
        );
        assert!(Instant::now() < deadline, "AGY task did not complete");
        thread::sleep(Duration::from_millis(50));
    }
}
