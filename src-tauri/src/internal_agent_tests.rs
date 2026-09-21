//! Regression tests for the intrinsic "Monitter Admin" agent. These cover the
//! schema migration (`internal: bool`), the one-shot bootstrap that creates
//! exactly one admin, and the centralized invariants enforced through
//! `Service::save_agent`, `Service::delete_agent`, task creation, channel
//! membership, collaboration discovery/routing, and extension eligibility.
//! The routing/broker for the resident admin lives in a later lane; this
//! file only guards the schema, bootstrap, and invariant surfaces.

use crate::{
    extensions, model::Channel, model::CreateTaskInput, model::INTERNAL_AGENT_NAME, Service,
};
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};

struct ServiceFixture {
    service: Arc<Service>,
    root: PathBuf,
}

impl Drop for ServiceFixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(name: &str) -> ServiceFixture {
    let root = std::env::temp_dir().join(format!("monitter-internal-agent-{name}-{}", crate::id()));
    let service = Service::open(None, root.clone()).unwrap();
    ServiceFixture { service, root }
}

fn agent_ids(service: &Arc<Service>) -> Vec<String> {
    service
        .snapshot()
        .unwrap()
        .agents
        .into_iter()
        .map(|agent| agent.id)
        .collect()
}

#[test]
fn old_agent_records_deserialize_with_internal_default_false() {
    // Older persisted state (no `internal` field) must continue to load.
    let agent: crate::model::Agent = serde_json::from_value(json!({
        "id":"a", "name":"Saved", "description":"", "instructions":"", "provider":"codex",
        "model":"", "hostId":"h", "cwd":"/tmp", "color":"#3f9d6a", "sandbox":"read-only"
    }))
    .unwrap();
    assert!(!agent.internal);
    assert!(!crate::model::is_internal_agent(&agent));
}

#[test]
fn default_snapshot_does_not_seed_an_internal_admin() {
    // `default_snapshot()` is the persisted default. The bootstrap migration
    // is responsible for appending the admin at runtime, never `default_snapshot`.
    let snapshot = crate::model::default_snapshot();
    assert!(
        snapshot.agents.iter().all(|agent| !agent.internal),
        "default_snapshot must not pre-mark any agent as internal"
    );
    assert_eq!(snapshot.agents.len(), 1);
}

#[test]
fn service_open_appends_exactly_one_monitter_admin_after_existing_agents() {
    let fixture = fixture("bootstrap-create");
    let snapshot = fixture.service.snapshot().unwrap();
    let internals: Vec<&crate::model::Agent> = snapshot
        .agents
        .iter()
        .filter(|agent| agent.internal)
        .collect();
    assert_eq!(
        internals.len(),
        1,
        "bootstrap must create exactly one internal agent"
    );
    let admin = internals[0];
    assert_eq!(admin.name, INTERNAL_AGENT_NAME);
    assert!(!admin.collaboration_enabled);
    assert_eq!(admin.provider, "codex");
    // The bootstrap appended, so the original default Codex agent is still
    // at index zero — earlier callers that hard-coded `agents[0]` stay stable.
    assert!(!snapshot.agents[0].internal);
    assert_eq!(snapshot.agents.last().unwrap().id, admin.id);
}

#[test]
fn bootstrap_is_idempotent_across_repeated_opens() {
    let root = std::env::temp_dir().join(format!("monitter-internal-agent-idem-{}", crate::id()));
    let service = Service::open(None, root.clone()).unwrap();
    let first = agent_ids(&service);
    let admin_id = service
        .internal_admin()
        .expect("first open should create the admin")
        .id;
    drop(service);
    let service = Service::open(None, root.clone()).unwrap();
    let second = agent_ids(&service);
    assert_eq!(
        first, second,
        "second open must not duplicate the resident admin"
    );
    let admin_again = service.internal_admin().expect("admin must persist");
    assert_eq!(admin_again.id, admin_id);
    service.cleanup();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn service_open_with_no_hosts_and_no_agents_leaves_admin_unconfigured() {
    // Synthesize a snapshot with no agents and no hosts so the bootstrap
    // cannot pick a source to clone. Persist it manually, then re-open.
    let root = std::env::temp_dir().join(format!("monitter-internal-agent-empty-{}", crate::id()));
    let mut snapshot = crate::model::default_snapshot();
    snapshot.agents.clear();
    snapshot.hosts.clear();
    let json = serde_json::to_string(&snapshot).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("state.json"), json).unwrap();
    let service = Service::open(None, root.clone()).unwrap();
    match service.internal_admin() {
        Err(error) => {
            assert!(
                error.contains("Monitter Admin is not configured"),
                "unexpected error: {error}"
            );
        }
        Ok(_) => panic!("admin must remain unconfigured when no host exists"),
    }
    assert!(
        service
            .snapshot()
            .unwrap()
            .agents
            .iter()
            .all(|agent| !agent.internal),
        "bootstrap must not fabricate an internal agent"
    );
    service.cleanup();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn internal_admin_reports_readable_error_when_persisted_state_has_multiple() {
    // Persist a snapshot that already contains two internal agents. The
    // bootstrap must surface a readable configuration error rather than
    // silently pick one.
    let root = std::env::temp_dir().join(format!("monitter-internal-agent-multi-{}", crate::id()));
    let mut snapshot = crate::model::default_snapshot();
    snapshot.agents.push(crate::model::Agent {
        id: crate::id(),
        internal: true,
        ..snapshot.agents[0].clone()
    });
    snapshot.agents.push(crate::model::Agent {
        id: crate::id(),
        internal: true,
        ..snapshot.agents[0].clone()
    });
    let json = serde_json::to_string(&snapshot).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("state.json"), json).unwrap();
    let service = Service::open(None, root.clone()).unwrap();
    let error = service
        .internal_admin()
        .expect_err("multiple internal agents must surface a configuration error");
    assert!(
        error.contains("Multiple Monitter Admin agents"),
        "unexpected error: {error}"
    );
    service.cleanup();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn save_agent_rejects_creating_a_new_internal_record() {
    let fixture = fixture("save-reject-new-internal");
    let mut agent = fixture.service.snapshot().unwrap().agents[0].clone();
    agent.id = String::new();
    agent.internal = true;
    let error = fixture
        .service
        .save_agent(agent)
        .expect_err("only bootstrap may create an internal agent");
    assert!(
        error.contains("bootstrap migration"),
        "unexpected error: {error}"
    );
    assert_eq!(
        fixture
            .service
            .snapshot()
            .unwrap()
            .agents
            .iter()
            .filter(|agent| agent.internal)
            .count(),
        1,
        "rejected save must not add another internal agent"
    );
}

#[test]
fn save_agent_preserves_internal_identity_when_editing_the_admin() {
    let fixture = fixture("save-keep-internal");
    let admin_id = fixture.service.internal_admin().unwrap().id;
    let mut edited = fixture.service.internal_admin().unwrap();
    edited.name = "Monitter Admin (renamed)".into();
    edited.description = "trying to rename".into();
    edited.collaboration_enabled = true;
    edited.internal = false;
    fixture
        .service
        .save_agent(edited)
        .expect("editing the admin must succeed");
    let admin = fixture.service.internal_admin().unwrap();
    assert_eq!(admin.id, admin_id);
    assert_eq!(admin.name, INTERNAL_AGENT_NAME, "rename must be reverted");
    assert!(admin.internal);
    assert!(
        !admin.collaboration_enabled,
        "admin collaboration must stay disabled"
    );
}

#[test]
fn save_agent_preserves_internal_identity_when_caller_omits_internal_flag() {
    let fixture = fixture("save-keep-internal-default");
    let mut edited = fixture.service.internal_admin().unwrap();
    edited.model = "gpt-test".into();
    edited.internal = false;
    fixture
        .service
        .save_agent(edited)
        .expect("edit without the flag must still preserve internal identity");
    let admin = fixture.service.internal_admin().unwrap();
    assert!(admin.internal, "internal flag must be re-applied on edit");
    assert_eq!(admin.name, INTERNAL_AGENT_NAME);
}

#[test]
fn save_agent_normalizes_the_default_agent_without_touching_internal_flag() {
    let fixture = fixture("save-default-agent");
    let mut edited = fixture.service.snapshot().unwrap().agents[0].clone();
    edited.model = "updated-model".into();
    // A normal agent edit that tries to flip `internal: true` must be
    // rejected because the existing record is not internal. The caller is
    // not allowed to promote an existing user agent into the resident admin.
    edited.internal = true;
    let err = fixture
        .service
        .save_agent(edited)
        .expect_err("promoting an existing normal agent to internal must fail");
    assert!(
        err.contains("bootstrap migration"),
        "unexpected error: {err}"
    );
    // Now make a clean edit without the illegal flag and confirm it succeeds
    // while leaving the internal flag alone.
    let mut clean = fixture.service.snapshot().unwrap().agents[0].clone();
    let original_id = clean.id.clone();
    clean.model = "updated-model".into();
    fixture
        .service
        .save_agent(clean)
        .expect("normal agent edits must succeed");
    let agents = fixture.service.snapshot().unwrap().agents;
    let updated = agents
        .iter()
        .find(|agent| agent.id == original_id)
        .expect("updated default agent must remain at the same id");
    assert_eq!(updated.model, "updated-model");
    assert!(!updated.internal);
}

#[test]
fn delete_agent_rejects_the_internal_admin() {
    let fixture = fixture("delete-internal");
    let admin_id = fixture.service.internal_admin().unwrap().id;
    let error = fixture
        .service
        .delete_agent(&admin_id, crate::DeleteAgentChatHandling::Archive)
        .expect_err("delete_agent must refuse to remove the admin");
    assert!(
        error.contains("Monitter Admin agent cannot be deleted"),
        "unexpected error: {error}"
    );
    assert!(
        fixture
            .service
            .snapshot()
            .unwrap()
            .agents
            .iter()
            .any(|agent| agent.id == admin_id),
        "rejected delete must leave the admin in place"
    );
}

#[test]
fn delete_agent_still_removes_normal_agents() {
    let fixture = fixture("delete-normal");
    let normal_id = fixture.service.snapshot().unwrap().agents[0].id.clone();
    fixture
        .service
        .delete_agent(&normal_id, crate::DeleteAgentChatHandling::Archive)
        .expect("normal agent deletion must still work");
    let agents = fixture.service.snapshot().unwrap().agents;
    assert!(agents.iter().all(|agent| agent.id != normal_id));
    // The internal admin must still be present.
    assert!(agents.iter().any(|agent| agent.internal));
}

#[test]
fn create_task_rejects_the_internal_admin_as_chat_recipient() {
    let fixture = fixture("create-task-internal");
    let admin_id = fixture.service.internal_admin().unwrap().id;
    let error = fixture
        .service
        .create_task(CreateTaskInput {
            agent_id: admin_id,
            title: "should fail".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .expect_err("create_task must refuse the admin");
    assert!(
        error.contains("Monitter Admin agent cannot be selected as a chat recipient"),
        "unexpected error: {error}"
    );
    assert!(
        fixture.service.snapshot().unwrap().tasks.is_empty(),
        "rejected task must not be persisted"
    );
}

#[test]
fn channel_membership_rejects_the_internal_admin() {
    let fixture = fixture("channel-membership");
    let admin_id = fixture.service.internal_admin().unwrap().id;
    let normal_id = fixture.service.snapshot().unwrap().agents[0].id.clone();
    fixture
        .service
        .mutate(None, |snapshot| {
            snapshot.channels.push(Channel {
                id: "team".into(),
                name: "Team".into(),
                description: String::new(),
                agent_ids: vec![normal_id.clone()],
                messages: vec![],
                agent_conversation_enabled: false,
                agent_conversation_turn_limit: 6,
                agent_conversation_turns_used: 0,
                agent_conversation_paused: false,
            });
            Ok(())
        })
        .unwrap();
    let error = fixture
        .service
        .set_channel_membership("team", &admin_id, true)
        .expect_err("joining a channel as the admin must fail");
    assert!(
        error.contains("Monitter Admin agent cannot join a channel"),
        "unexpected error: {error}"
    );
    let channels = fixture.service.snapshot().unwrap().channels;
    assert!(
        channels[0]
            .agent_ids
            .iter()
            .all(|member| member != &admin_id),
        "admin must not be added to the channel"
    );
}

#[test]
fn save_channel_rejects_internal_admin_in_desired_membership() {
    let fixture = fixture("save-channel-internal");
    let admin_id = fixture.service.internal_admin().unwrap().id;
    let normal_id = fixture.service.snapshot().unwrap().agents[0].id.clone();
    let error = fixture
        .service
        .save_channel(Channel {
            id: "needs-admin".into(),
            name: "Needs admin".into(),
            description: String::new(),
            agent_ids: vec![normal_id, admin_id],
            messages: vec![],
            agent_conversation_enabled: false,
            agent_conversation_turn_limit: 6,
            agent_conversation_turns_used: 0,
            agent_conversation_paused: false,
        })
        .expect_err("save_channel must reject the admin");
    assert!(
        error.contains("reserved Monitter Admin"),
        "unexpected error: {error}"
    );
}

#[test]
fn collaboration_directory_excludes_the_internal_admin() {
    let fixture = fixture("collab-directory");
    let admin_id = fixture.service.internal_admin().unwrap().id;
    let root_task_id = {
        let admin_id = admin_id.clone();
        let caller = fixture.service.snapshot().unwrap().agents[0].clone();
        let admin = fixture.service.internal_admin().unwrap().clone();
        let task = fixture
            .service
            .create_task(CreateTaskInput {
                agent_id: caller.id.clone(),
                title: "caller".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        fixture
            .service
            .mutate(None, |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == task.id)
                    .unwrap()
                    .status = "running".into();
                snapshot
                    .agents
                    .iter_mut()
                    .find(|agent| agent.id == admin_id)
                    .unwrap()
                    .collaboration_enabled = true;
                Ok(())
            })
            .unwrap();
        let _ = admin;
        task.id
    };
    let directory = fixture
        .service
        .protocol(&root_task_id, "list_agents", json!({}))
        .unwrap();
    let agents = directory["agents"].as_array().unwrap();
    assert!(
        agents
            .iter()
            .all(|entry| entry["id"].as_str() != Some(fixture.service.internal_admin().unwrap().id.as_str())),
        "directory must never include the admin, even when its collaboration flag is enabled: {:#?}",
        agents
    );
}

#[test]
fn collaboration_routing_rejects_the_internal_admin_as_recipient() {
    let fixture = fixture("collab-routing");
    let admin_id = fixture.service.internal_admin().unwrap().id.clone();
    let caller = fixture.service.snapshot().unwrap().agents[0].clone();
    let task = fixture
        .service
        .create_task(CreateTaskInput {
            agent_id: caller.id.clone(),
            title: "caller".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    fixture
        .service
        .mutate(None, |snapshot| {
            snapshot
                .tasks
                .iter_mut()
                .find(|t| t.id == task.id)
                .unwrap()
                .status = "running".into();
            snapshot
                .agents
                .iter_mut()
                .find(|agent| agent.id == caller.id)
                .unwrap()
                .collaboration_enabled = true;
            snapshot
                .agents
                .iter_mut()
                .find(|agent| agent.id == admin_id)
                .unwrap()
                .collaboration_enabled = true;
            Ok(())
        })
        .unwrap();
    let error = fixture
        .service
        .protocol(
            &task.id,
            "delegate_task",
            json!({
                "to_agent_id": admin_id,
                "title": "review",
                "message": "please",
                "request_id": "r1"
            }),
        )
        .expect_err("delegate_task to the admin must fail");
    assert!(
        error.contains("Recipient agent is unavailable"),
        "unexpected error: {error}"
    );
}

#[test]
fn extension_eligible_set_excludes_the_internal_admin() {
    let fixture = fixture("extension-eligible");
    let admin_id = fixture.service.internal_admin().unwrap().id.clone();
    let extensions = fixture.service.extension_config().expect("load extensions");
    assert!(
        extensions.mcp_servers.is_empty(),
        "no MCP servers are configured yet"
    );
    // Save an MCP server that targets the admin — the normalize layer must
    // reject the reference rather than silently allowing it.
    let result = fixture
        .service
        .save_extension_config(extensions::ExtensionConfig {
            revision: extensions.revision.clone(),
            mcp_servers: vec![extensions::McpServerConfig {
                id: "11111111-1111-4111-8111-111111111111".into(),
                name: "Admin only".into(),
                enabled: true,
                agent_ids: vec![admin_id.clone()],
                transport: extensions::McpTransport::Stdio,
                command: "echo".into(),
                args: vec!["hi".into()],
                env: Default::default(),
                url: String::new(),
                headers: Default::default(),
            }],
            skills: vec![],
        });
    let err = match result {
        Ok(_) => panic!("admin must not be an eligible extension agent"),
        Err(error) => error,
    };
    assert!(
        err.contains("Extension agent selections")
            || err.contains("Monitter Admin")
            || err.contains("agent"),
        "unexpected error: {err}"
    );
}

#[test]
fn lan_invoke_save_agent_routes_through_centralized_validation() {
    let fixture = fixture("lan-save-agent");
    let admin_id = fixture.service.internal_admin().unwrap().id.clone();
    let mut edited = fixture.service.internal_admin().unwrap().clone();
    edited.name = "Wrong name".into();
    edited.internal = false;
    let result = fixture
        .service
        .lan_invoke("save_agent", json!({"agent": edited}));
    assert!(result.is_ok(), "centralized save must succeed: {result:?}");
    let snapshot: crate::model::Snapshot =
        serde_json::from_value(result.unwrap()).expect("snapshot payload");
    let admin = snapshot
        .agents
        .iter()
        .find(|agent| agent.id == admin_id)
        .expect("admin must still be present");
    assert_eq!(admin.name, INTERNAL_AGENT_NAME);
    assert!(admin.internal);
}

#[test]
fn lan_invoke_delete_agent_rejects_the_internal_admin() {
    let fixture = fixture("lan-delete-agent");
    let admin_id = fixture.service.internal_admin().unwrap().id.clone();
    let error = fixture
        .service
        .lan_invoke("delete_agent", json!({"id": admin_id}))
        .expect_err("LAN delete must refuse the admin");
    assert!(
        error.contains("Monitter Admin agent cannot be deleted"),
        "unexpected error: {error}"
    );
}

fn user_agent(fixture: &ServiceFixture) -> crate::model::Agent {
    fixture
        .service
        .snapshot()
        .unwrap()
        .agents
        .into_iter()
        .find(|agent| !agent.internal)
        .expect("the fixture must seed at least one user agent")
}

#[test]
fn delete_agent_default_archives_tasks_and_preserves_agent_and_llm_reference() {
    let fixture = fixture("delete-archive-default");
    let agent = user_agent(&fixture);
    let original_name = agent.name.clone();
    let original_provider = agent.provider.clone();
    let original_model = agent.model.clone();
    let first = fixture
        .service
        .create_task(crate::model::CreateTaskInput {
            agent_id: agent.id.clone(),
            title: "Keep me".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    let second = fixture
        .service
        .create_task(crate::model::CreateTaskInput {
            agent_id: agent.id.clone(),
            title: "Keep me too".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    // Seed a queued follow-up and a pending approval so we can assert they
    // are cleaned up while the underlying tasks are archived.
    fixture
        .service
        .mutate(None, |snapshot| {
            snapshot.queued_messages.push(crate::model::QueuedMessage {
                id: "queued-1".into(),
                task_id: first.id.clone(),
                channel_id: None,
                text: "follow-up".into(),
                attachment_ids: vec![],
                created_at: 0,
                status: "queued".into(),
                error: None,
                sender_agent_id: None,
                origin: None,
            });
            snapshot.approval_requests.push(crate::model::ApprovalRequest {
                id: "approval-1".into(),
                task_id: second.id.clone(),
                provider: original_provider.clone(),
                run_id: "run-1".into(),
                tool: "shell".into(),
                summary: "rm -rf".into(),
                detail: String::new(),
                risk: "high".into(),
                status: "pending".into(),
                created_at: 0,
                resolved_at: None,
                decision: None,
                input: None,
                response: None,
                rememberable: false,
                session_scope: None,
                rule_id: None,
                approval_scope: None,
            });
            Ok(())
        })
        .unwrap();

    fixture
        .service
        .delete_agent(&agent.id, crate::DeleteAgentChatHandling::Archive)
        .expect("default archive must succeed");

    let snapshot = fixture.service.snapshot().unwrap();
    assert!(
        snapshot.agents.iter().all(|a| a.id != agent.id),
        "agent must be removed"
    );
    let archived: Vec<&crate::model::Task> = snapshot
        .tasks
        .iter()
        .filter(|t| t.id == first.id || t.id == second.id)
        .collect();
    assert_eq!(archived.len(), 2, "both chats must remain in the snapshot");
    for task in archived {
        assert!(task.archived, "task must be archived: {task:?}");
        assert_eq!(
            task.archived_agent_name.as_deref(),
            Some(original_name.as_str()),
            "agent display name must be captured so the chat stays labelled"
        );
        assert_eq!(task.provider, original_provider);
        assert_eq!(task.model, original_model);
    }
    assert!(
        snapshot
            .queued_messages
            .iter()
            .filter(|m| m.task_id == first.id || m.task_id == second.id)
            .all(|m| m.status == "error" && m.error.as_deref() == Some("Owner agent removed.")),
        "queued follow-ups for the removed agent must be failed with an owner-removal reason"
    );
    assert!(
        snapshot
            .approval_requests
            .iter()
            .filter(|r| r.task_id == first.id || r.task_id == second.id)
            .all(|r| r.status == "expired"),
        "approval requests for the removed agent must be expired"
    );
}

#[test]
fn delete_agent_with_delete_handling_drops_tasks_and_history() {
    let fixture = fixture("delete-chats");
    let agent = user_agent(&fixture);
    let task = fixture
        .service
        .create_task(crate::model::CreateTaskInput {
            agent_id: agent.id.clone(),
            title: "Erase me".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    fixture
        .service
        .mutate(None, |snapshot| {
            snapshot.messages.push(crate::model::Message {
                stream_status: None,
                phase: None,
                response_metadata: None,
                id: "m-1".into(),
                task_id: task.id.clone(),
                role: "user".into(),
                text: "hello".into(),
                created_at: 0,
                sender_agent_id: None,
                collaboration_id: None,
                attachments: vec![],
            });
            Ok(())
        })
        .unwrap();

    fixture
        .service
        .delete_agent(&agent.id, crate::DeleteAgentChatHandling::Delete)
        .expect("delete mode must succeed");

    let snapshot = fixture.service.snapshot().unwrap();
    assert!(
        snapshot.agents.iter().all(|a| a.id != agent.id),
        "agent must be removed"
    );
    assert!(
        snapshot.tasks.iter().all(|t| t.id != task.id),
        "task must be removed in delete mode"
    );
    assert!(
        snapshot.messages.iter().all(|m| m.task_id != task.id),
        "messages for the deleted task must be removed"
    );
    assert!(
        snapshot.events.iter().all(|e| e.task_id != task.id),
        "activity events for the deleted task must be removed"
    );
}

#[test]
fn delete_agent_with_running_task_cancels_then_archives() {
    let fixture = fixture("delete-running");
    let agent = user_agent(&fixture);
    let task = fixture
        .service
        .create_task(crate::model::CreateTaskInput {
            agent_id: agent.id.clone(),
            title: "running".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    fixture
        .service
        .mutate(None, |snapshot| {
            let task = snapshot
                .tasks
                .iter_mut()
                .find(|t| t.id == task.id)
                .unwrap();
            task.status = "running".into();
            Ok(())
        })
        .unwrap();

    fixture
        .service
        .delete_agent(&agent.id, crate::DeleteAgentChatHandling::Archive)
        .expect("archive of an agent with a running task must succeed");

    let snapshot = fixture.service.snapshot().unwrap();
    let archived = snapshot
        .tasks
        .iter()
        .find(|t| t.id == task.id)
        .expect("task must remain archived in the snapshot");
    assert!(archived.archived);
    assert_eq!(
        archived.status, "interrupted",
        "running task must be cancelled before being archived"
    );
    assert_eq!(
        archived.archived_agent_name.as_deref(),
        Some(agent.name.as_str()),
        "agent reference must still be captured after cancellation"
    );
    let system_message = snapshot
        .messages
        .iter()
        .rev()
        .find(|m| m.task_id == task.id && m.role == "system" && m.text.contains("owning agent was removed"))
        .expect("cancellation must leave an owner-removal system boundary message");
    assert_eq!(system_message.role, "system");
    let status_event = snapshot
        .events
        .iter()
        .rev()
        .find(|event| event.task_id == task.id && event.title.contains("owning agent was removed"))
        .expect("diagnostic timeline must record the owner-removal status");
    assert_eq!(status_event.kind, "status");
}

#[test]
fn delete_agent_with_unknown_handling_value_rejects_visibly() {
    let fixture = fixture("delete-bad-handling");
    let agent = user_agent(&fixture);
    let error = fixture
        .service
        .lan_invoke("delete_agent", json!({"id": agent.id, "chatHandling": "nuke"})) // (del intended)
        .expect_err("unknown chatHandling must be rejected");
    assert!(
        error.contains("Unknown chat handling mode"),
        "unexpected error: {error}"
    );
    assert!(
        fixture
            .service
            .snapshot()
            .unwrap()
            .agents
            .iter()
            .any(|a| a.id == agent.id),
        "rejected delete must leave the agent untouched"
    );
}

#[test]
fn lan_invoke_delete_agent_archives_by_default() {
    let fixture = fixture("lan-delete-default");
    let agent = user_agent(&fixture);
    let task = fixture
        .service
        .create_task(crate::model::CreateTaskInput {
            agent_id: agent.id.clone(),
            title: "lan-default".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap();
    fixture
        .service
        .lan_invoke("delete_agent", json!({"id": agent.id}))
        .expect("LAN delete with no chatHandling must default to archive");
    let snapshot = fixture.service.snapshot().unwrap();
    assert!(
        snapshot.agents.iter().all(|a| a.id != agent.id),
        "agent must be removed through the LAN path too"
    );
    let archived = snapshot
        .tasks
        .iter()
        .find(|t| t.id == task.id)
        .expect("task must survive when archive is the default");
    assert!(archived.archived);
    assert_eq!(
        archived.archived_agent_name.as_deref(),
        Some(agent.name.as_str())
    );
}
