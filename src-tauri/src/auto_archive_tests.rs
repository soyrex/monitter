use super::*;
use crate::model::{
    default_snapshot, Collaboration, Message, Project, QueuedMessage, ScheduleRun,
    ScheduleRunStatus, Task,
};
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};

const DAY: i64 = 86_400_000;
const NOW: i64 = 2_000 * DAY;

struct Fixture {
    service: Arc<Service>,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.cleanup();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("monitter-auto-archive-{name}-{}", crate::id()));
    let service = Service::open(None, root.clone()).expect("open isolated service");
    Fixture { service, root }
}

fn task(service: &Service, id: &str, project_id: Option<&str>) -> Task {
    let snapshot = service.snapshot().unwrap();
    let agent = &snapshot.agents[0];
    Task {
        id: id.into(),
        agent_id: agent.id.clone(),
        title: id.into(),
        native_session_id: None,
        status: "idle".into(),
        archived: false,
        created_at: NOW - 10 * DAY,
        updated_at: NOW - 10 * DAY,
        parent_task_id: None,
        channel_id: None,
        host_id: agent.host_id.clone(),
        cwd: agent.cwd.clone(),
        provider: agent.provider.clone(),
        codex_home: None,
        model: agent.model.clone(),
        model_settings: None,
        sandbox: "workspace-write".into(),
        project_id: project_id.map(str::to_owned),
        acp: None,
        archived_agent_name: None, agent_worktree: None,
    }
}

fn caller(service: &Service) -> String {
    let mut item = task(service, "caller", None);
    item.status = "running".into();
    let id = item.id.clone();
    service
        .mutate(None, |snapshot| {
            snapshot.tasks.push(item);
            Ok(())
        })
        .unwrap();
    id
}

fn install_tasks(service: &Service, tasks: Vec<Task>) {
    service
        .mutate(None, |snapshot| {
            snapshot.tasks.extend(tasks);
            Ok(())
        })
        .unwrap();
}

fn rule(project_id: Option<&str>, days: u32, enabled: bool) -> ChatArchiveRule {
    ChatArchiveRule {
        project_id: project_id.map(str::to_owned),
        older_than_days: days,
        enabled,
        created_at: NOW,
        updated_at: NOW,
    }
}

#[test]
fn selection_is_strict_at_cutoff_and_uses_latest_message_activity() {
    let f = fixture("strict-cutoff");
    let mut at_cutoff = task(&f.service, "at-cutoff", None);
    at_cutoff.updated_at = NOW - DAY;
    let stale = task(&f.service, "stale", None);
    let mut recent_message = task(&f.service, "recent-message", None);
    recent_message.updated_at = NOW - 20 * DAY;
    install_tasks(&f.service, vec![at_cutoff, stale, recent_message]);
    f.service
        .mutate(None, |snapshot| {
            snapshot.messages.push(Message {
                id: "message".into(),
                task_id: "recent-message".into(),
                role: "assistant".into(),
                text: "later".into(),
                created_at: NOW - DAY / 2,
                stream_status: None,
                phase: None,
                response_metadata: None,
                sender_agent_id: None,
                collaboration_id: None,
                attachments: vec![],
            });
            Ok(())
        })
        .unwrap();
    let snapshot = f.service.snapshot().unwrap();
    let matches = select(&snapshot, &HashSet::new(), Some((None, 1)), NOW);
    assert_eq!(
        matches
            .matches
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["stale"]
    );
}

#[test]
fn recurring_rules_use_project_override_and_keep_unscoped_global_tasks() {
    let f = fixture("rule-precedence");
    let project_id = uuid::Uuid::new_v4().to_string();
    let project = Project { build_management_enabled: false, agent_worktrees_enabled: false,
        id: project_id.clone(),
        name: "Project".into(),
        description: String::new(),
        icon: "folder".into(),
        color: "#000".into(),
        workspaces: vec![],
        board_guidance_text: None,
    };
    install_tasks(
        &f.service,
        vec![
            task(&f.service, "global", None),
            task(&f.service, "project", Some(&project_id)),
        ],
    );
    f.service
        .mutate(None, |snapshot| {
            snapshot.projects.push(project);
            snapshot.chat_archive_rules =
                vec![rule(None, 1, true), rule(Some(&project_id), 1, false)];
            Ok(())
        })
        .unwrap();
    assert_eq!(f.service.auto_archive_at(NOW).unwrap(), 1);
    let snapshot = f.service.snapshot().unwrap();
    assert!(
        snapshot
            .tasks
            .iter()
            .find(|t| t.id == "global")
            .unwrap()
            .archived
    );
    assert!(
        !snapshot
            .tasks
            .iter()
            .find(|t| t.id == "project")
            .unwrap()
            .archived
    );
}

#[test]
fn selection_protects_all_active_categories_and_ancestors() {
    let f = fixture("protections");
    let ids = [
        "unknown", "resident", "stream", "queue", "approval", "collab", "subagent", "plan",
        "schedule", "cleanup", "internal", "parent", "child",
    ];
    let mut tasks = ids
        .iter()
        .map(|id| task(&f.service, id, None))
        .collect::<Vec<_>>();
    tasks.iter_mut().find(|t| t.id == "unknown").unwrap().status = "mystery".into();
    tasks
        .iter_mut()
        .find(|t| t.id == "child")
        .unwrap()
        .parent_task_id = Some("parent".into());
    install_tasks(&f.service, tasks);
    f.service.mutate(None, |s| {
        let mut internal_agent = s.agents[0].clone();
        internal_agent.id = "internal-agent".into();
        internal_agent.internal = true;
        s.agents.push(internal_agent);
        s.tasks.iter_mut().find(|t| t.id == "internal").unwrap().agent_id = "internal-agent".into();
        s.messages.push(Message { id: "m".into(), task_id: "stream".into(), role: "assistant".into(), text: String::new(), created_at: NOW - 9 * DAY, stream_status: Some("streaming".into()), phase: None, response_metadata: None, sender_agent_id: None, collaboration_id: None, attachments: vec![] });
        s.queued_messages.push(QueuedMessage { id: "q".into(), task_id: "queue".into(), channel_id: None, text: String::new(), attachment_ids: vec![], created_at: 0, status: "sending".into(), error: None, sender_agent_id: None, origin: None });
        s.approval_requests.push(serde_json::from_value(json!({"id":"a","taskId":"approval","provider":"codex","runId":"r","tool":"x","summary":"x","detail":"x","risk":"x","status":"pending","createdAt":0})).unwrap());
        s.collaborations.push(Collaboration { model_selection: None, id: "c".into(), kind: "delegation".into(), from_agent_id: "a".into(), from_task_id: "collab".into(), to_agent_id: "b".into(), to_task_id: "child".into(), text: String::new(), request_id: "r".into(), status: "running".into(), result: None, error: None, created_at: 0, updated_at: 0 });
        s.subagent_sessions.push(serde_json::from_value(json!({"id":"sub","source":"codex","parentTaskId":"subagent","status":"running","createdAt":0,"updatedAt":0})).unwrap());
        s.work_plans.push(serde_json::from_value(json!({"id":"p","taskId":"plan","requestId":"r","title":"p","status":"active","items":[],"createdAt":0,"updatedAt":0,"closedAt":null})).unwrap());
        s.pending_throwaway_task_ids.push("cleanup".into());
        s.schedules.push(serde_json::from_value(json!({"id":"s","title":"s","agentId":s.agents[0].id,"prompt":"p","frequency":"0 0 * * *","tz":"host","mode":"persistent_thread","enabled":false,"createdAt":0,"updatedAt":0})).unwrap());
        s.schedule_runs.push(ScheduleRun { id: "run".into(), schedule_id: "s".into(), fired_at_ms: 0, mode: ScheduleMode::PersistentThread, task_id: Some("schedule".into()), status: ScheduleRunStatus::Succeeded, finished_at_ms: Some(0), summary: String::new(), error: None, skipped_overlap: false });
        Ok(())
    }).unwrap();
    let snapshot = f.service.snapshot().unwrap();
    let residents = HashSet::from(["resident"]);
    let selection = select(&snapshot, &residents, Some((None, 1)), NOW);
    assert!(selection.matches.is_empty());
    assert_eq!(selection.skipped.values().sum::<usize>(), ids.len());
    assert_eq!(selection.skipped.get("active_descendant"), Some(&1));
}

#[test]
fn protocol_preview_does_not_mutate_apply_preserves_history_and_unarchive_works() {
    let f = fixture("preview-apply");
    let caller = caller(&f.service);
    install_tasks(&f.service, vec![task(&f.service, "old", None)]);
    f.service
        .mutate(None, |s| {
            s.messages.push(Message {
                id: "history".into(),
                task_id: "old".into(),
                role: "user".into(),
                text: "keep".into(),
                created_at: NOW - 9 * DAY,
                stream_status: None,
                phase: None,
                response_metadata: None,
                sender_agent_id: None,
                collaboration_id: None,
                attachments: vec![],
            });
            Ok(())
        })
        .unwrap();
    let preview = f
        .service
        .chat_archive_protocol(
            &caller,
            "archive_chats",
            &serde_json::from_value(json!({"scope":"global","older_than_days":1})).unwrap(),
        )
        .unwrap();
    assert_eq!(preview["dryRun"], true);
    assert!(
        !f.service
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "old")
            .unwrap()
            .archived
    );
    let applied = f
        .service
        .chat_archive_protocol(
            &caller,
            "archive_chats",
            &serde_json::from_value(json!({"scope":"global","older_than_days":1,"dry_run":false}))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(applied["archivedCount"], 1);
    let after = f.service.snapshot().unwrap();
    assert!(after.tasks.iter().find(|t| t.id == "old").unwrap().archived);
    assert_eq!(
        after.messages.iter().filter(|m| m.task_id == "old").count(),
        1
    );
    f.service.set_task_archived("old", false).unwrap();
    assert!(
        !f.service
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "old")
            .unwrap()
            .archived
    );
}

#[test]
fn protocol_apply_rechecks_preview_candidates_after_new_activity() {
    let f = fixture("apply-recheck");
    let caller = caller(&f.service);
    install_tasks(&f.service, vec![task(&f.service, "old", None)]);
    let preview = f
        .service
        .protocol(
            &caller,
            "archive_chats",
            json!({"scope":"global","older_than_days":1}),
        )
        .unwrap();
    assert_eq!(preview["matchedCount"], 1);
    f.service
        .mutate(None, |s| {
            s.queued_messages.push(QueuedMessage {
                id: "new-queue".into(),
                task_id: "old".into(),
                channel_id: None,
                text: "new work".into(),
                attachment_ids: vec![],
                created_at: NOW,
                status: "queued".into(),
                error: None,
                sender_agent_id: None,
                origin: None,
            });
            Ok(())
        })
        .unwrap();
    let applied = f
        .service
        .protocol(
            &caller,
            "archive_chats",
            json!({"scope":"global","older_than_days":1,"dry_run":false}),
        )
        .unwrap();
    assert_eq!(applied["archivedCount"], 0);
    assert!(
        !f.service
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .find(|task| task.id == "old")
            .unwrap()
            .archived
    );
}

#[test]
fn protocol_one_time_project_scope_excludes_other_projects_and_unassigned_tasks() {
    let f = fixture("one-time-project");
    let caller = caller(&f.service);
    let included = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    f.service
        .mutate(None, |s| {
            for (id, name) in [(&included, "Included"), (&other, "Other")] {
                s.projects.push(Project { build_management_enabled: false, agent_worktrees_enabled: false,
                    id: (*id).clone(),
                    name: name.into(),
                    description: String::new(),
                    icon: "folder".into(),
                    color: "#000".into(),
                    workspaces: vec![],
                    board_guidance_text: None,
                });
            }
            Ok(())
        })
        .unwrap();
    install_tasks(
        &f.service,
        vec![
            task(&f.service, "included", Some(&included)),
            task(&f.service, "other", Some(&other)),
            task(&f.service, "unassigned", None),
        ],
    );
    let result = f
        .service
        .protocol(
            &caller,
            "archive_chats",
            json!({"scope":"project","project_id":included,"older_than_days":1,"dry_run":false}),
        )
        .unwrap();
    assert_eq!(result["archivedCount"], 1);
    let snapshot = f.service.snapshot().unwrap();
    assert!(
        snapshot
            .tasks
            .iter()
            .find(|t| t.id == "included")
            .unwrap()
            .archived
    );
    assert!(
        !snapshot
            .tasks
            .iter()
            .find(|t| t.id == "other")
            .unwrap()
            .archived
    );
    assert!(
        !snapshot
            .tasks
            .iter()
            .find(|t| t.id == "unassigned")
            .unwrap()
            .archived
    );
}

#[test]
fn protocol_enforces_permissions_disabled_collaboration_and_invalid_inputs() {
    let f = fixture("authorization");
    let caller = caller(&f.service);
    f.service
        .mutate(None, |s| {
            s.tasks.iter_mut().find(|t| t.id == caller).unwrap().sandbox = "read-only".into();
            Ok(())
        })
        .unwrap();
    let preview: serde_json::Map<String, Value> =
        serde_json::from_value(json!({"scope":"global","older_than_days":1})).unwrap();
    assert!(f
        .service
        .chat_archive_protocol(&caller, "archive_chats", &preview)
        .is_ok());
    let write: serde_json::Map<String, Value> =
        serde_json::from_value(json!({"scope":"global","older_than_days":1,"dry_run":false}))
            .unwrap();
    assert!(f
        .service
        .chat_archive_protocol(&caller, "archive_chats", &write)
        .unwrap_err()
        .contains("Read-only"));
    f.service
        .mutate(None, |s| {
            s.tasks.iter_mut().find(|t| t.id == caller).unwrap().sandbox = "workspace-write".into();
            s.agents[0].collaboration_enabled = false;
            Ok(())
        })
        .unwrap();
    assert!(f
        .service
        .chat_archive_protocol(&caller, "list_chat_archive_rules", &serde_json::Map::new())
        .unwrap_err()
        .contains("not eligible"));
    assert!(validate_args(
        "archive_chats",
        &json!({"scope":"project","project_id":"not-a-uuid","older_than_days":0})
    )
    .is_err());
    assert!(validate_args(
        "archive_chats",
        &json!({"scope":"global","project_id":uuid::Uuid::new_v4().to_string(),"older_than_days":1})
    )
    .is_err());
    f.service
        .mutate(None, |s| {
            s.agents[0].collaboration_enabled = true;
            Ok(())
        })
        .unwrap();
    let missing: serde_json::Map<String, Value> = serde_json::from_value(json!({"scope":"project","project_id":uuid::Uuid::new_v4().to_string(),"older_than_days":1})).unwrap();
    assert!(f
        .service
        .chat_archive_protocol(&caller, "archive_chats", &missing)
        .unwrap_err()
        .contains("Project was not found"));
}

#[test]
fn project_rule_can_be_saved_then_deleted_to_restore_inheritance() {
    let f = fixture("rule-delete");
    let caller = caller(&f.service);
    let project_id = uuid::Uuid::new_v4().to_string();
    f.service
        .mutate(None, |s| {
            s.projects.push(Project { build_management_enabled: false, agent_worktrees_enabled: false,
                id: project_id.clone(),
                name: "Project".into(),
                description: String::new(),
                icon: "folder".into(),
                color: "#000".into(),
                workspaces: vec![],
                board_guidance_text: None,
            });
            Ok(())
        })
        .unwrap();
    f.service
        .mutate(None, |s| {
            s.chat_archive_rules = vec![
                rule(Some(&project_id), 1, true),
                rule(Some(&project_id), 2, false),
            ];
            Ok(())
        })
        .unwrap();
    let saved: serde_json::Map<String, Value> = serde_json::from_value(
        json!({"scope":"project","project_id":project_id,"older_than_days":3,"enabled":false}),
    )
    .unwrap();
    f.service
        .chat_archive_protocol(&caller, "save_chat_archive_rule", &saved)
        .unwrap();
    let rules = &f.service.snapshot().unwrap().chat_archive_rules;
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].older_than_days, 3);
    let replacement: serde_json::Map<String, Value> = serde_json::from_value(
        json!({"scope":"project","project_id":project_id,"older_than_days":4}),
    )
    .unwrap();
    f.service
        .chat_archive_protocol(&caller, "save_chat_archive_rule", &replacement)
        .unwrap();
    let rules = &f.service.snapshot().unwrap().chat_archive_rules;
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].older_than_days, 4);
    let deleted: serde_json::Map<String, Value> =
        serde_json::from_value(json!({"scope":"project","project_id":project_id})).unwrap();
    let result = f
        .service
        .chat_archive_protocol(&caller, "delete_chat_archive_rule", &deleted)
        .unwrap();
    assert_eq!(result["removed"], true);
    assert!(f.service.snapshot().unwrap().chat_archive_rules.is_empty());
}

#[test]
fn rules_persist_across_sqlite_reopen() {
    let root =
        std::env::temp_dir().join(format!("monitter-auto-archive-persistence-{}", crate::id()));
    {
        let service = Service::open(None, root.clone()).unwrap();
        let caller = caller(&service);
        let args: serde_json::Map<String, Value> =
            serde_json::from_value(json!({"scope":"global","older_than_days":14,"enabled":false}))
                .unwrap();
        service
            .chat_archive_protocol(&caller, "save_chat_archive_rule", &args)
            .unwrap();
        service.cleanup();
    }
    let reopened = Service::open(None, root.clone()).unwrap();
    let rules = reopened.snapshot().unwrap().chat_archive_rules;
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].project_id, None);
    assert_eq!(rules[0].older_than_days, 14);
    assert!(!rules[0].enabled);
    reopened.cleanup();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn legacy_snapshot_without_rules_defaults_to_empty() {
    let mut value = serde_json::to_value(default_snapshot()).unwrap();
    value.as_object_mut().unwrap().remove("chatArchiveRules");
    let loaded: Snapshot = serde_json::from_value(value).unwrap();
    assert!(loaded.chat_archive_rules.is_empty());
}
