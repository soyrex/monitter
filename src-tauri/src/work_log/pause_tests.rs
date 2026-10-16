//! Disposable primary/destination fixtures for paused collection boundaries.
//! Declare from lib.rs with #[path = "work_log/pause_tests.rs"] mod work_log_pause_tests;
use crate::{
    model::{ApprovalRequest, Message},
    work_log::{
        self,
        access::{CallerIdentity, GrantScope, Operation, OwnerGrant, SourceIdentity},
        *,
    },
    *,
};
use std::{path::PathBuf, sync::Arc};

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("monitter-pause-{}", id())))
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn enabled() -> CollectionSettings {
    CollectionSettings {
        enabled: true,
        ..Default::default()
    }
}
fn test_task(service: &Arc<Service>) -> Task {
    let agent = service.snapshot().unwrap().agents[0].id.clone();
    service
        .create_task(CreateTaskInput {
            agent_id: agent,
            title: "Pause fixture".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        })
        .unwrap()
}
fn message(task: &str, role: &str, text: &str, at: i64) -> Message {
    Message {
        id: id(),
        task_id: task.into(),
        role: role.into(),
        text: text.into(),
        created_at: at,
        sender_agent_id: None,
        collaboration_id: None,
        stream_status: None,
        phase: None,
        response_metadata: None,
        attachments: vec![],
    }
}
fn begin(service: &Arc<Service>, task: &Task) -> Arc<runner::RunControl> {
    service
        .mutate(None, |s| {
            s.tasks.iter_mut().find(|t| t.id == task.id).unwrap().status = "running".into();
            Ok(())
        })
        .unwrap();
    let control = service.reserve_run(&task.id).unwrap();
    service.work_log_register_run(&task.id, &control);
    control
}
fn flush(service: &Service, profile: &Profile) -> work_log::store::WorkLogDb {
    let db = work_log::store::WorkLogDb::open(&profile.0).unwrap();
    work_log::writer::Writer::new(&db)
        .flush(&service.store, 200)
        .unwrap();
    db
}

#[test]
fn pause_keeps_actual_run_and_freezes_context_then_resume_uses_new_segment() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(enabled()).unwrap();
    let task = test_task(&service);
    let control = begin(&service, &task);
    let run_id = control.current_run_id().unwrap();
    let before = message(&task.id, "assistant", "before pause", now());
    service
        .mutate(None, |s| {
            s.messages.push(before.clone());
            Ok(())
        })
        .unwrap();
    let old_ref = service
        .store
        .work_log_active_context(&run_id)
        .unwrap()
        .unwrap()
        .context_ref;
    service
        .work_log_configure(CollectionSettings::default())
        .unwrap();
    let frozen = service
        .store
        .work_log_active_context(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        frozen.through_message_id.as_deref(),
        Some(before.id.as_str())
    );
    assert_eq!(
        frozen.through_message_text_bytes,
        Some(before.text.len() as u32)
    );
    assert_eq!(service.store.work_log_active().unwrap()[0].run_id, run_id);
    assert!(service.store.work_log_active().unwrap()[0]
        .outcome
        .is_none());
    let hidden = message(&task.id, "assistant", "text during pause", now());
    service
        .mutate(None, |s| {
            s.messages.push(hidden.clone());
            Ok(())
        })
        .unwrap();
    service.work_log_configure(enabled()).unwrap();
    let resumed = service
        .store
        .work_log_active_context(&run_id)
        .unwrap()
        .unwrap();
    assert_ne!(resumed.context_ref, old_ref);
    assert!(resumed.source_message_id.is_none());
    let after = message(&task.id, "assistant", "after resume", now() + 2);
    service
        .mutate(None, |s| {
            s.messages.push(after.clone());
            Ok(())
        })
        .unwrap();
    let anchored = service
        .store
        .work_log_active_context(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        anchored.segment_start_id.as_deref(),
        Some(after.id.as_str())
    );
    assert_eq!(
        anchored.through_message_id.as_deref(),
        Some(after.id.as_str())
    );
    assert_eq!(
        anchored.through_message_text_bytes,
        Some(after.text.len() as u32)
    );
    let grown = "after resume with later streamed text";
    service
        .mutate(None, |s| {
            s.messages
                .iter_mut()
                .find(|m| m.id == after.id)
                .unwrap()
                .text = grown.into();
            Ok(())
        })
        .unwrap();
    let history = rusqlite::Connection::open_with_flags(
        profile.0.join("state.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let source = SourceIdentity {
        installation_id: anchored.installation_id.clone(),
        profile_id: anchored.profile_id.clone(),
        task_id: task.id.clone(),
        project_id: anchored.project_id.clone(),
        lineage_id: anchored.lineage_root_task_id.clone(),
    };
    let caller = CallerIdentity {
        installation_id: anchored.installation_id.clone(),
        profile_id: anchored.profile_id.clone(),
        task_id: "reader".into(),
        run_id: "reader-run".into(),
    };
    let grant = OwnerGrant {
        installation_id: anchored.installation_id.clone(),
        profile_id: anchored.profile_id.clone(),
        caller_task_id: caller.task_id.clone(),
        operation: Operation::ReadContext,
        scope: GrantScope::Task(task.id.clone()),
        revoked: false,
    };
    let provisional = work_log::context::read_page(
        &history,
        &anchored,
        &source,
        &caller,
        || Ok(vec![grant.clone()]),
        None,
        Some(64),
    )
    .unwrap();
    assert_eq!(provisional.messages.len(), 1);
    assert_eq!(provisional.messages[0].text, after.text);
    service
        .mutate(None, |s| {
            s.tasks.iter_mut().find(|t| t.id == task.id).unwrap().status = "completed".into();
            Ok(())
        })
        .unwrap();
    let db = flush(&service, &profile);
    let old = db.context_ref_lookup(&old_ref).unwrap().unwrap();
    let new = db
        .context_ref_lookup(&anchored.context_ref)
        .unwrap()
        .unwrap();
    assert_eq!(old.through_message_id.as_deref(), Some(before.id.as_str()));
    assert_eq!(new.segment_start_id.as_deref(), Some(after.id.as_str()));
    assert_eq!(new.through_message_id.as_deref(), Some(after.id.as_str()));
    assert_eq!(new.through_message_text_bytes, Some(grown.len() as u32));
    let final_page = work_log::context::read_page(
        &history,
        &new,
        &source,
        &caller,
        || Ok(vec![grant.clone()]),
        None,
        Some(64),
    )
    .unwrap();
    assert_eq!(final_page.messages[0].text, grown);
    let stored = db
        .connection()
        .unwrap()
        .runs()
        .unwrap()
        .into_iter()
        .find(|r| r.run_id == run_id)
        .unwrap();
    assert_eq!(stored.outcome.as_deref(), Some("completed"));
    let connection = rusqlite::Connection::open_with_flags(
        profile.0.join("work-log.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (total,open):(i64,i64)=connection.query_row("SELECT count(*),sum(CASE WHEN ended_at IS NULL THEN 1 ELSE 0 END) FROM collector_gaps WHERE reason='collectionPaused'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((total, open), (1, 0));
    service.release_run(&task.id);
}

#[test]
fn provider_request_ids_form_one_wait_union() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(enabled()).unwrap();
    let task = test_task(&service);
    let control = begin(&service, &task);
    let run = control.current_run_id().unwrap();
    let make = |provider_id: &str| ApprovalRequest {
        id: id(),
        task_id: task.id.clone(),
        provider: task.provider.clone(),
        run_id: provider_id.into(),
        tool: "Tool".into(),
        summary: "Needs approval".into(),
        detail: "fixture".into(),
        risk: "unknown".into(),
        status: "pending".into(),
        created_at: now(),
        resolved_at: None,
        decision: None,
        rememberable: false,
        session_scope: None,
        rule_id: None,
        approval_scope: None,
        input: None,
        response: None,
    };
    let first = make("acp-permission:call-1");
    let second = make("codex-rpc-call-2");
    service
        .mutate(None, |s| {
            s.approval_requests.push(first.clone());
            Ok(())
        })
        .unwrap();
    service
        .mutate(None, |s| {
            s.approval_requests.push(second.clone());
            Ok(())
        })
        .unwrap();
    service
        .mutate(None, |s| {
            s.approval_requests
                .iter_mut()
                .find(|a| a.id == first.id)
                .unwrap()
                .status = "approved".into();
            Ok(())
        })
        .unwrap();
    service
        .mutate(None, |s| {
            s.approval_requests
                .iter_mut()
                .find(|a| a.id == second.id)
                .unwrap()
                .status = "approved".into();
            Ok(())
        })
        .unwrap();
    let db = flush(&service, &profile);
    let connection = rusqlite::Connection::open_with_flags(
        profile.0.join("work-log.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM work_intervals WHERE run_id=?1 AND kind='approvalWait'",
            [run],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "overlapping provider requests produce one wait interval"
    );
    service.release_run(&task.id);
}

#[test]
fn task_ending_during_pause_closes_at_last_observed_boundary() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(enabled()).unwrap();
    let task = test_task(&service);
    let control = begin(&service, &task);
    let run = control.current_run_id().unwrap();
    service
        .work_log_configure(CollectionSettings::default())
        .unwrap();
    let boundary = service.store.work_log_active().unwrap()[0].observed_at;
    service
        .mutate(None, |s| {
            s.tasks.iter_mut().find(|t| t.id == task.id).unwrap().status = "completed".into();
            Ok(())
        })
        .unwrap();
    assert!(service.store.work_log_active().unwrap().is_empty());
    let db = flush(&service, &profile);
    let stored = db
        .connection()
        .unwrap()
        .runs()
        .unwrap()
        .into_iter()
        .find(|r| r.run_id == run)
        .unwrap();
    assert_eq!(stored.outcome.as_deref(), Some("unknownEndDuringPause"));
    assert_eq!(stored.ended_at, Some(boundary));
    assert!(!service.store.work_log_settings().unwrap().enabled);
    service.release_run(&task.id);
}

#[test]
fn disabled_recovery_uses_frozen_context_and_does_not_enable_capture() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(enabled()).unwrap();
    let task = test_task(&service);
    let control = begin(&service, &task);
    let run = control.current_run_id().unwrap();
    let before = message(&task.id, "assistant", "accepted", now());
    service
        .mutate(None, |s| {
            s.messages.push(before.clone());
            Ok(())
        })
        .unwrap();
    service
        .work_log_configure(CollectionSettings::default())
        .unwrap();
    let frozen = service
        .store
        .work_log_active_context(&run)
        .unwrap()
        .unwrap();
    let after = message(&task.id, "assistant", "while disabled", now());
    service
        .mutate(None, |s| {
            s.messages.push(after);
            Ok(())
        })
        .unwrap();
    service.work_log_recover();
    assert!(!service.store.work_log_settings().unwrap().enabled);
    assert!(service.store.work_log_active().unwrap().is_empty());
    let db = flush(&service, &profile);
    let stored = db
        .connection()
        .unwrap()
        .runs()
        .unwrap()
        .into_iter()
        .find(|r| r.run_id == run)
        .unwrap();
    assert_eq!(stored.outcome.as_deref(), Some("unknownEndDuringPause"));
    let context = db.context_ref_lookup(&frozen.context_ref).unwrap().unwrap();
    assert_eq!(
        context.through_message_id.as_deref(),
        Some(before.id.as_str())
    );
    service.release_run(&task.id);
}
