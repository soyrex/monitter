//! Focused tests for saved agent identity/profile prompt boundaries.

use crate::{
    agent_instructions, initial_task_instructions,
    model::{Channel, CreateTaskInput},
    Service,
};
use std::sync::Arc;
use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

struct TestService {
    service: Arc<Service>,
    root: PathBuf,
}

impl Deref for TestService {
    type Target = Arc<Service>;
    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

impl Drop for TestService {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn service(name: &str) -> TestService {
    let root = std::env::temp_dir().join(format!("monitter-agent-identity-{name}-{}", crate::id()));
    let service = Service::open(None, root.clone()).unwrap();
    TestService { service, root }
}

fn task(service: &Arc<Service>, channel_id: Option<String>) -> crate::Task {
    let agent = service
        .snapshot()
        .unwrap()
        .agents
        .into_iter()
        .find(|agent| agent.provider == "codex")
        .unwrap();
    service
        .create_task(CreateTaskInput {
            agent_id: agent.id,
            title: "identity test".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id,
            project_id: None,
            cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
            model_settings: None,
            sandbox: Some("read-only".into()),
        })
        .unwrap()
}

#[test]
fn identity_contains_user_and_complete_saved_profile() {
    let mut agent = crate::model::default_snapshot().agents[0].clone();
    agent.name = "Planner".into();
    agent.description = "Plans releases".into();
    agent.expertise = vec!["Rust".into()];
    agent.responsibilities = vec!["Review changes".into()];
    agent.skills = vec!["Testing".into()];
    agent.instructions = "Be precise.".into();
    let text = agent_instructions(&agent, "Alex");
    assert!(text.starts_with("You are acting as Planner. You are an agent running inside the Monitter harness. Your user is \"Alex\"."));
    for expected in [
        "Agent settings and instructions:",
        "Purpose: Plans releases",
        "Expertise:\n- Rust",
        "Responsibilities:\n- Review changes",
        "Skills:\n- Testing",
        "Be precise.",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
}

#[test]
fn direct_first_prompt_uses_saved_identity_once_and_keeps_user_message_unchanged() {
    let service = service("direct");
    service
        .mutate(None, |snapshot| {
            snapshot.settings.user_name = "Alex".into();
            let agent = snapshot
                .agents
                .iter_mut()
                .find(|agent| agent.provider == "codex")
                .unwrap();
            agent.name = "Planner".into();
            agent.description = "Plans releases".into();
            agent.instructions.clear();
            Ok(())
        })
        .unwrap();
    let task = task(&service, None);
    let prompt = service
        .accept_send(task.id.clone(), "  ship it  ".into(), vec![])
        .unwrap()
        .unwrap();
    assert!(prompt.contains("You are acting as Planner"));
    assert!(prompt.contains("Agent settings and instructions:"));
    assert!(prompt.ends_with("User request:\nship it"));
    let stored = service
        .snapshot()
        .unwrap()
        .messages
        .into_iter()
        .find(|message| message.task_id == task.id && message.role == "user")
        .unwrap();
    assert_eq!(stored.text, "ship it");

    service
        .mutate(None, |snapshot| {
            snapshot
                .tasks
                .iter_mut()
                .find(|item| item.id == task.id)
                .unwrap()
                .status = "completed".into();
            snapshot
                .agents
                .iter_mut()
                .find(|agent| agent.provider == "codex")
                .unwrap()
                .name = "Changed".into();
            snapshot.settings.user_name = "Other".into();
            Ok(())
        })
        .unwrap();
    let later = service
        .accept_send(task.id, "follow up".into(), vec![])
        .unwrap()
        .unwrap();
    assert_eq!(later, "follow up");
}

#[test]
fn channel_first_and_queued_first_turns_retain_profile_when_freeform_instructions_empty() {
    let service = service("channel");
    service
        .mutate(None, |snapshot| {
            snapshot.settings.user_name = "Alex".into();
            let agent = snapshot
                .agents
                .iter_mut()
                .find(|agent| agent.provider == "codex")
                .unwrap();
            agent.description = "Coordinates channels".into();
            agent.expertise = vec!["Routing".into()];
            agent.responsibilities = vec!["Keep context".into()];
            agent.skills = vec!["Summarising".into()];
            agent.instructions.clear();
            let agent_id = agent.id.clone();
            snapshot.channels.push(Channel {
                id: "channel-test".into(),
                name: "Team".into(),
                description: String::new(),
                agent_ids: vec![agent_id],
                messages: vec![],
                agent_conversation_enabled: false,
                agent_conversation_turn_limit: 3,
                agent_conversation_turns_used: 0,
                agent_conversation_paused: false,
            });
            Ok(())
        })
        .unwrap();
    let first = task(&service, Some("channel-test".into()));
    let snapshot = service.snapshot().unwrap();
    let profile = initial_task_instructions(&snapshot, &first.id).unwrap();
    assert!(profile.contains("You are acting as Codex"));
    assert!(profile.contains("Purpose: Coordinates channels"));
    assert!(profile.contains("Expertise:\n- Routing"));

    // A queued first-turn task has the same empty transcript invariant; the
    // queue drain uses this exact helper before composing its channel prompt.
    let queued = task(&service, Some("channel-test".into()));
    let snapshot = service.snapshot().unwrap();
    let queued_profile = initial_task_instructions(&snapshot, &queued.id).unwrap();
    assert_eq!(queued_profile, profile);
}

fn opencode_fixture(capture: &std::path::Path) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("monitter-opencode-fixture-{}", crate::id()));
    let script = format!(
        "#!/bin/sh\ncat > {}\nprintf '%s\\n' '{{\"type\":\"text\",\"part\":{{\"type\":\"text\",\"text\":\"fixture reply\",\"sessionID\":\"fixture-session\"}}}}'\n",
        crate::runner::posix_quote(&capture.to_string_lossy())
    );
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    path
}

struct HarnessFixture {
    service: TestService,
    capture: PathBuf,
    executable: PathBuf,
}

impl Drop for HarnessFixture {
    fn drop(&mut self) {
        // The service must release its owned fake harness before its executable
        // and capture are removed, including when an assertion panics.
        self.service.cleanup();
        let _ = fs::remove_file(&self.capture);
        let _ = fs::remove_file(&self.executable);
    }
}

fn wait_capture(service: &Service, task_id: &str, path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let finished = service.snapshot().ok().and_then(|snapshot| {
            snapshot
                .tasks
                .into_iter()
                .find(|task| task.id == task_id)
                .map(|task| task.status)
        });
        if finished.as_deref() == Some("completed") {
            if let Ok(value) = fs::read_to_string(path) {
                if !value.is_empty() {
                    return value;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "fixture did not finish with a captured channel prompt: {finished:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn channel_delivery_passes_complete_profile_to_real_harness_prompt() {
    check_channel_delivery("Keep replies concise.");
}

#[test]
fn channel_delivery_passes_profile_when_freeform_instructions_are_empty() {
    check_channel_delivery("");
}

fn check_channel_delivery(instructions: &str) {
    let capture = std::env::temp_dir().join(format!("monitter-channel-prompt-{}", crate::id()));
    let executable = opencode_fixture(&capture);
    let fixture = HarnessFixture {
        service: service("channel-harness"),
        capture,
        executable,
    };
    let service = &fixture.service;
    let workspace = fixture.service.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let (agent_id, channel_id) = {
        let snapshot = service.snapshot().unwrap();
        (snapshot.agents[0].id.clone(), "channel-harness".to_string())
    };
    service
        .mutate(None, |snapshot| {
            snapshot.settings.user_name = "Alex".into();
            let host_id = snapshot
                .agents
                .iter()
                .find(|agent| agent.id == agent_id)
                .unwrap()
                .host_id
                .clone();
            let host = snapshot
                .hosts
                .iter_mut()
                .find(|host| host.id == host_id)
                .unwrap();
            host.kind = "local".into();
            host.default_cwd = workspace.to_string_lossy().into();
            host.opencode_path = fixture.executable.to_string_lossy().into();
            let agent = snapshot
                .agents
                .iter_mut()
                .find(|agent| agent.id == agent_id)
                .unwrap();
            agent.provider = "opencode".into();
            agent.sandbox = "harness-configured".into();
            agent.cwd = workspace.to_string_lossy().into();
            agent.collaboration_enabled = false;
            agent.name = "Channel Planner".into();
            agent.description = "Coordinates channels".into();
            agent.expertise = vec!["Routing".into()];
            agent.responsibilities = vec!["Keep context".into()];
            agent.skills = vec!["Summarising".into()];
            agent.instructions = instructions.into();
            snapshot.channels.push(Channel {
                id: channel_id.clone(),
                name: "Team".into(),
                description: String::new(),
                agent_ids: vec![agent_id.clone()],
                messages: vec![],
                agent_conversation_enabled: false,
                agent_conversation_turn_limit: 3,
                agent_conversation_turns_used: 0,
                agent_conversation_paused: false,
            });
            Ok(())
        })
        .unwrap();
    crate::send_channel_message_accepted(
        service.service.clone(),
        channel_id.clone(),
        "channel request".into(),
        vec![agent_id.clone()],
        None,
    )
    .unwrap();
    let task_id = service
        .snapshot()
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| {
            task.channel_id.as_deref() == Some(channel_id.as_str()) && task.agent_id == agent_id
        })
        .unwrap()
        .id;
    let prompt = wait_capture(service, &task_id, &fixture.capture);
    for expected in [
        "You are acting as Channel Planner",
        "Your user is \"Alex\"",
        "Agent settings and instructions:",
        "Purpose: Coordinates channels",
        "Expertise:\n- Routing",
        "Responsibilities:\n- Keep context",
        "Skills:\n- Summarising",
        instructions,
        "New message:\nchannel request",
    ] {
        assert!(
            prompt.contains(expected),
            "missing {expected:?} in {prompt}"
        );
    }
}

#[test]
fn queued_channel_first_turn_passes_complete_profile_to_real_harness_prompt() {
    let capture =
        std::env::temp_dir().join(format!("monitter-queued-channel-prompt-{}", crate::id()));
    let executable = opencode_fixture(&capture);
    let fixture = HarnessFixture {
        service: service("queued-channel-harness"),
        capture,
        executable,
    };
    let service = &fixture.service;
    let workspace = fixture.service.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let (agent_id, host_id) = {
        let snapshot = service.snapshot().unwrap();
        (
            snapshot.agents[0].id.clone(),
            snapshot.agents[0].host_id.clone(),
        )
    };
    service
        .mutate(None, |snapshot| {
            snapshot.settings.user_name = "Alex".into();
            let host = snapshot
                .hosts
                .iter_mut()
                .find(|host| host.id == host_id)
                .unwrap();
            host.kind = "local".into();
            host.default_cwd = workspace.to_string_lossy().into();
            host.opencode_path = fixture.executable.to_string_lossy().into();
            let agent = snapshot
                .agents
                .iter_mut()
                .find(|agent| agent.id == agent_id)
                .unwrap();
            agent.provider = "opencode".into();
            agent.sandbox = "harness-configured".into();
            agent.cwd = workspace.to_string_lossy().into();
            agent.collaboration_enabled = false;
            agent.name = "Channel Planner".into();
            agent.description = "Coordinates channels".into();
            agent.expertise = vec!["Routing".into()];
            agent.responsibilities = vec!["Keep context".into()];
            agent.skills = vec!["Summarising".into()];
            agent.instructions = "Keep replies concise.".into();
            snapshot.channels.push(Channel {
                id: "queued-channel".into(),
                name: "Team".into(),
                description: String::new(),
                agent_ids: vec![agent_id.clone()],
                messages: vec![],
                agent_conversation_enabled: false,
                agent_conversation_turn_limit: 3,
                agent_conversation_turns_used: 0,
                agent_conversation_paused: false,
            });
            Ok(())
        })
        .unwrap();
    let queued_task = service
        .create_task(CreateTaskInput {
            agent_id: agent_id.clone(),
            title: "queued first".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: Some("queued-channel".into()),
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: Some("harness-configured".into()),
        })
        .unwrap();
    service
        .mutate(None, |snapshot| {
            snapshot.queued_messages.push(crate::model::QueuedMessage {
                id: crate::id(),
                task_id: queued_task.id.clone(),
                channel_id: Some("queued-channel".into()),
                text: "queued request".into(),
                attachment_ids: vec![],
                created_at: crate::now(),
                status: "queued".into(),
                error: None,
                sender_agent_id: None,
                origin: None,
            });
            Ok(())
        })
        .unwrap();
    service.dispatch_queued(&queued_task.id);
    let prompt = wait_capture(service, &queued_task.id, &fixture.capture);
    for expected in [
        "You are acting as Channel Planner",
        "Your user is \"Alex\"",
        "Agent settings and instructions:",
        "Purpose: Coordinates channels",
        "Expertise:\n- Routing",
        "Responsibilities:\n- Keep context",
        "Skills:\n- Summarising",
        "Keep replies concise.",
        "New message:\nqueued request",
    ] {
        assert!(
            prompt.contains(expected),
            "missing {expected:?} in {prompt}"
        );
    }
}
