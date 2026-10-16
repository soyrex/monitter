//! Disposable-profile acceptance fixtures; no provider invocation or live data.
use crate::{
    work_log::{access::*, *},
    *,
};
struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("monitter-work-log-{}", id())))
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn message(task: &str, role: &str, text: &str) -> Message {
    Message {
        id: id(),
        task_id: task.into(),
        role: role.into(),
        text: text.into(),
        created_at: now(),
        sender_agent_id: None,
        collaboration_id: None,
        stream_status: None,
        phase: None,
        response_metadata: None,
        attachments: vec![],
    }
}
fn task(service: &Arc<Service>, title: &str) -> Task {
    let agent = service.snapshot().unwrap().agents[0].id.clone();
    service
        .create_task(CreateTaskInput {
            agent_id: agent,
            title: title.into(),
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
fn begin(service: &Arc<Service>, task: &Task) -> Arc<runner::RunControl> {
    service
        .mutate(None, |s| {
            s.tasks.iter_mut().find(|t| t.id == task.id).unwrap().status = "running".into();
            s.agents
                .iter_mut()
                .find(|a| a.id == task.agent_id)
                .unwrap()
                .collaboration_enabled = true;
            Ok(())
        })
        .unwrap();
    let control = service.reserve_run(&task.id).unwrap();
    service.work_log_register_run(&task.id, &control);
    control
}
fn flush(service: &Service, db: &work_log::store::WorkLogDb) {
    work_log::writer::Writer::new(db)
        .flush(&service.store, 200)
        .unwrap();
}
fn settings() -> CollectionSettings {
    CollectionSettings {
        enabled: true,
        ..Default::default()
    }
}

#[test]
fn work_log_prospective_run_context_replay_and_fresh_agent() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    assert!(!service.store.work_log_settings().unwrap().enabled);
    service.work_log_configure(settings()).unwrap();
    let source = task(&service, "Original title");
    let first = message(&source.id, "user", "Persist this source question");
    service
        .mutate(None, |s| {
            s.messages.push(first.clone());
            Ok(())
        })
        .unwrap();
    let control = begin(&service, &source);
    let run_id = control.current_run_id().unwrap();
    assert!(service
        .lan_invoke(
            "work_log_owner",
            serde_json::json!({"operation":"health","args":{}})
        )
        .is_err());
    let answer = message(&source.id, "assistant", "Persist this source answer");
    service
        .mutate(None, |s| {
            s.messages.push(answer.clone());
            s.tasks
                .iter_mut()
                .find(|t| t.id == source.id)
                .unwrap()
                .status = "completed".into();
            Ok(())
        })
        .unwrap();
    service.release_run(&source.id);
    let db = work_log::store::WorkLogDb::open(&profile.0).unwrap();
    flush(&service, &db);
    let run = db
        .connection()
        .unwrap()
        .runs()
        .unwrap()
        .into_iter()
        .find(|r| r.run_id == run_id)
        .unwrap();
    assert_eq!(run.outcome.as_deref(), Some("completed"));
    assert!(run.elapsed_ms.is_some());
    let reference = run.context_ref.clone();
    // Rename/archive do not retarget either identity or context anchors.
    service
        .mutate(None, |s| {
            let t = s.tasks.iter_mut().find(|t| t.id == source.id).unwrap();
            t.title = "Renamed".into();
            t.archived = true;
            Ok(())
        })
        .unwrap();
    let reader = task(&service, "Fresh authorized reader");
    let reader_control = begin(&service, &reader);
    let (install, private) = service.store.work_log_identity().unwrap();
    let grants = [Operation::ReadContext, Operation::ReadLog]
        .into_iter()
        .map(|operation| OwnerGrant {
            installation_id: install.clone(),
            profile_id: private.clone(),
            caller_task_id: reader.id.clone(),
            operation,
            scope: GrantScope::Task(source.id.clone()),
            revoked: false,
        })
        .collect();
    service.work_log_set_grants(grants).unwrap();
    let discovered = service
        .work_log_query_protocol(
            &reader.id,
            WorkQuery {
                task_id: Some(source.id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(discovered["entries"][0]["run"]["contextRef"], reference);
    assert_eq!(discovered["entries"][0]["run"]["title"], "Original title");
    let page = service
        .work_log_read_context_protocol(&reader.id, &reference, None, Some(1))
        .unwrap();
    assert_eq!(page["messages"][0]["text"], first.text);
    let cursor = page["nextCursor"].as_str().unwrap();
    let next = service
        .work_log_read_context_protocol(&reader.id, &reference, Some(cursor), Some(64))
        .unwrap();
    assert_eq!(next["messages"][0]["text"], answer.text);
    service.work_log_set_grants(vec![]).unwrap();
    assert!(service
        .work_log_read_context_protocol(&reader.id, &reference, Some(cursor), None)
        .is_err());
    flush(&service, &db);
    let count = db.connection().unwrap().runs().unwrap().len();
    flush(&service, &db);
    assert_eq!(db.connection().unwrap().runs().unwrap().len(), count);
    drop(reader_control);
    drop(control);
    drop(db);
    drop(service);
    let reopened = Service::open(None, profile.0.clone()).unwrap();
    let db = work_log::store::WorkLogDb::open(&profile.0).unwrap();
    flush(&reopened, &db);
    let old = db
        .connection()
        .unwrap()
        .runs()
        .unwrap()
        .into_iter()
        .find(|r| r.run_id == run_id)
        .unwrap();
    assert_eq!(old.title, "Original title");
    assert_eq!(old.context_ref, reference);
    assert_eq!(old.outcome.as_deref(), Some("completed"));
}

#[test]
fn work_log_usage_survives_staged_deletion_and_reopen() {
    let profile = Profile::new();
    let (store, snapshot, hosts, attachments) =
        crate::store::Store::open(profile.0.clone()).unwrap();
    store.work_log_configure(&settings()).unwrap();
    let sample = RunUsageSample {
        sample_id: "usage-deleted".into(),
        run_id: "run-deleted".into(),
        task_id: "task-deleted".into(),
        provider: "codex".into(),
        configured_model: None,
        started_at: 1,
        observed_at: 2,
        final_sample: false,
        classification: "cumulative".into(),
        counter_epoch: None,
        provider_turn_id: None,
        tokens: UsageTokens {
            input: Some(17),
            ..Default::default()
        },
        cost_usd: None,
        duration_ms: None,
        api_duration_ms: None,
        provider_turns: None,
        context: None,
    };
    store.stage_usage(sample.clone()).unwrap();
    store.remove_task_usage(&sample.task_id).unwrap();
    store.save(&snapshot, &hosts, &attachments).unwrap();
    drop(store);
    let (store, _, _, _) = crate::store::Store::open(profile.0.clone()).unwrap();
    let pending = store.pending(100).unwrap();
    assert!(pending
        .iter()
        .any(|r| matches!(&r.record,WorkRecord::Usage(s)if s.sample_id==sample.sample_id)));
    let db = work_log::store::WorkLogDb::open(&profile.0).unwrap();
    work_log::writer::Writer::new(&db)
        .flush(&store, 100)
        .unwrap();
    assert_eq!(
        db.connection().unwrap().usage_observations().unwrap().len(),
        1
    );
    assert!(store.pending(100).unwrap().is_empty());
}

#[test]
fn work_log_outbox_rollback_capacity_pause_and_report_identity() {
    let profile = Profile::new();
    std::fs::create_dir_all(&profile.0).unwrap();
    let mut primary = rusqlite::Connection::open(profile.0.join("primary.sqlite3")).unwrap();
    primary.execute_batch(work_log::outbox::DDL).unwrap();
    work_log::outbox::set_meta(
        &primary,
        "settings",
        &serde_json::to_string(&settings()).unwrap(),
    )
    .unwrap();
    let run = work_log::types::fixtures::run("r1");
    let record = PendingRecord {
        source_key: "start".into(),
        record: WorkRecord::Run(run.clone()),
    };
    {
        let tx = primary.transaction().unwrap();
        work_log::outbox::append(&tx, &[record.clone()], false).unwrap();
    }
    assert!(work_log::outbox::pending(&primary, 100).unwrap().is_empty());
    assert!(work_log::outbox::active(&primary).unwrap().is_empty());
    let input = ReportInput {
        request_id: "checkpoint".into(),
        phase: "implement".into(),
        summary: "fixture checkpoint".into(),
        plan_item_id: None,
        labels: vec![],
    };
    {
        let tx = primary.transaction().unwrap();
        let one = work_log::outbox::report(&tx, &run, &input).unwrap();
        let retry = work_log::outbox::report(&tx, &run, &input).unwrap();
        assert_eq!(one, retry);
        let mut changed = input.clone();
        changed.summary = "changed".into();
        assert!(work_log::outbox::report(&tx, &run, &changed).is_err());
        let mut another = run.clone();
        another.run_id = "r2".into();
        let receipt = work_log::outbox::report(&tx, &another, &input).unwrap();
        assert_ne!(one.source_key, receipt.source_key);
        tx.commit().unwrap();
    }
    primary.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO work_log_outbox(source_key,payload)SELECT 'fill-'||x,'null' FROM n").unwrap();
    {
        let tx = primary.transaction().unwrap();
        assert!(work_log::outbox::append(&tx, &[record], false)
            .unwrap()
            .is_empty());
        assert_eq!(work_log::outbox::active(&tx).unwrap().len(), 1);
        let mut terminal = run.clone();
        terminal.outcome = Some("collectionPaused".into());
        terminal.ended_at = Some(5);
        assert!(work_log::outbox::append(
            &tx,
            &[PendingRecord {
                source_key: "pause".into(),
                record: WorkRecord::Run(terminal)
            }],
            false
        )
        .unwrap()
        .is_empty());
        assert!(work_log::outbox::active(&tx).unwrap().is_empty());
        assert!(work_log::outbox::report(
            &tx,
            &run,
            &ReportInput {
                request_id: "full".into(),
                ..input
            }
        )
        .is_err());
        tx.commit().unwrap();
    }
    primary.execute("DELETE FROM work_log_outbox", []).unwrap();
    let tx = primary.transaction().unwrap();
    work_log::outbox::append(&tx, &[], false).unwrap();
    tx.commit().unwrap();
    let replay = work_log::outbox::pending(&primary, 100).unwrap();
    assert!(replay.iter().any(
        |r| matches!(&r.record,WorkRecord::Gap(g)if g.reason=="sourceOutboxCapacity"&&g.dropped>=2)
    ));
}

#[test]
fn work_log_wait_union_and_owner_deletion_does_not_resurrect_or_delete_chat() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(settings()).unwrap();
    let source = task(&service, "Wait fixture");
    let control = begin(&service, &source);
    let run = control.current_run_id().unwrap();
    let approval = |identity: &str| {
        serde_json::from_value::<ApprovalRequest>(serde_json::json!({"id":identity,"taskId":source.id,"provider":"fixture","runId":run,"tool":"fixture","summary":"fixture","detail":"private arguments must not enter history","risk":"low","status":"pending","createdAt":now()})).unwrap()
    };
    let first = approval("first");
    let second = approval("second");
    service
        .mutate(None, |s| {
            s.approval_requests.push(first);
            Ok(())
        })
        .unwrap();
    service
        .mutate(None, |s| {
            s.approval_requests.push(second);
            Ok(())
        })
        .unwrap();
    service
        .mutate(None, |s| {
            s.approval_requests[0].status = "approved".into();
            Ok(())
        })
        .unwrap();
    service
        .mutate(None, |s| {
            s.approval_requests[1].status = "approved".into();
            Ok(())
        })
        .unwrap();
    service
        .mutate(None, |s| {
            s.tasks
                .iter_mut()
                .find(|t| t.id == source.id)
                .unwrap()
                .status = "completed".into();
            Ok(())
        })
        .unwrap();
    service.release_run(&source.id);
    let pending = service.store.pending(100).unwrap();
    assert_eq!(
        pending
            .iter()
            .filter(|r| matches!(&r.record,WorkRecord::Interval(i)if i.kind=="approvalWait"))
            .count(),
        1
    );
    let db = work_log::store::WorkLogDb::open(&profile.0).unwrap();
    flush(&service, &db);
    assert!(
        !serde_json::to_string(&db.connection().unwrap().events().unwrap())
            .unwrap()
            .contains("private arguments")
    );
    assert_eq!(db.delete_runs(&[run.clone()]).unwrap(), 1);
    // A historical source transaction still waiting for acknowledgement may
    // replay after deletion. Tombstones acknowledge it without resurrecting it.
    db.apply_batch(&pending).unwrap();
    assert!(db.connection().unwrap().runs().unwrap().is_empty());
    assert!(service
        .snapshot()
        .unwrap()
        .tasks
        .iter()
        .any(|t| t.id == source.id));
}

#[test]
fn work_log_derived_root_survives_deleted_intermediate_and_reopen() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    let root = task(&service, "Root");
    let derived = |parent: &Task| {
        service
            .create_task(CreateTaskInput {
                agent_id: parent.agent_id.clone(),
                title: "Derived".into(),
                native_session_id: None,
                parent_task_id: Some(parent.id.clone()),
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap()
    };
    let middle = derived(&root);
    let leaf = derived(&middle);
    assert_eq!(
        service
            .store
            .work_log_lineage(&leaf.id)
            .unwrap()
            .3
            .as_deref(),
        Some(root.id.as_str())
    );
    service
        .mutate(None, |s| {
            s.tasks.retain(|t| t.id != middle.id);
            s.events.retain(|e| e.task_id != middle.id);
            Ok(())
        })
        .unwrap();
    drop(service);
    let reopened = Service::open(None, profile.0.clone()).unwrap();
    assert_eq!(
        reopened
            .store
            .work_log_lineage(&leaf.id)
            .unwrap()
            .3
            .as_deref(),
        Some(root.id.as_str())
    );
}

#[test]
fn work_log_background_worker_without_gui_flushes_reopens_and_accepts_private_controls() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(settings()).unwrap();
    service.start_work_log_collector();
    let source = task(&service, "Backend-only fixture");
    let control = begin(&service, &source);
    let run = control.current_run_id().unwrap();
    service
        .mutate(None, |s| {
            s.tasks
                .iter_mut()
                .find(|t| t.id == source.id)
                .unwrap()
                .status = "completed".into();
            Ok(())
        })
        .unwrap();
    service.release_run(&source.id);
    assert_eq!(
        service
            .work_log_owner("flush", serde_json::json!({}))
            .unwrap()["flushed"],
        true
    );
    let q = serde_json::json!({"taskId":source.id});
    let page = service.work_log_owner("query", q.clone()).unwrap();
    assert!(!page["entries"].as_array().unwrap().is_empty());
    let summary = service.work_log_owner("summary", q).unwrap();
    assert_eq!(summary["runs"], 1);
    service.stop_work_log_collector();
    drop(service);
    let reopened = Service::open(None, profile.0.clone()).unwrap();
    reopened.start_work_log_collector();
    assert_eq!(
        reopened
            .work_log_owner("deleteHistory", serde_json::json!({"runIds":[run]}))
            .unwrap()["deletedRuns"],
        1
    );
    assert!(reopened
        .snapshot()
        .unwrap()
        .tasks
        .iter()
        .any(|t| t.id == source.id));
    reopened.stop_work_log_collector();
}

#[test]
#[cfg(target_os = "macos")]
fn work_log_backend_worker_observes_real_owned_fixture_process_without_provider() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(settings()).unwrap();
    let source = task(&service, "Owned sleep fixture; no model call");
    let control = begin(&service, &source);
    let run = control.current_run_id().unwrap();
    struct ChildGuard(Arc<runner::RunControl>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            self.0.work_log_stop_fixture_child();
        }
    }
    let child = std::process::Command::new("/bin/sleep")
        .arg("30")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    control.work_log_fixture_child(child);
    let _guard = ChildGuard(control.clone());
    service.start_work_log_collector();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(18);
    let observed = loop {
        let value=service.work_log_reader().ok().and_then(|c|c.query_row("SELECT cpu_delta_ms,rss_bytes,covered_ms FROM resource_samples WHERE run_id=?1 AND quality='observed' LIMIT 1",[&run],|r|Ok((r.get::<_,Option<i64>>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,i64>(2)?))).ok());
        if let Some(value) = value {
            break value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "collector did not observe owned fixture process"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert!(observed.0.is_some_and(|cpu| cpu >= 0));
    assert!(observed.1.is_some_and(|rss| rss > 0));
    assert!(observed.2 > 0);
    service
        .mutate(None, |s| {
            s.tasks
                .iter_mut()
                .find(|t| t.id == source.id)
                .unwrap()
                .status = "completed".into();
            Ok(())
        })
        .unwrap();
    service.release_run(&source.id);
    service.stop_work_log_collector();
}

#[test]
fn work_log_export_shutdown_joins_cleanup_and_preserves_complete_artifacts() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(settings()).unwrap();
    let source = task(&service, "Export fixture");
    let _control = begin(&service, &source);
    service
        .mutate(None, |s| {
            s.tasks
                .iter_mut()
                .find(|t| t.id == source.id)
                .unwrap()
                .status = "completed".into();
            Ok(())
        })
        .unwrap();
    service.release_run(&source.id);
    let db = work_log::store::WorkLogDb::open(&profile.0).unwrap();
    flush(&service, &db);
    let run = db.connection().unwrap().runs().unwrap()[0].clone();
    let mut sequence = db.connection().unwrap().watermark().unwrap();
    for batch in 0..40 {
        let records = (0..50)
            .map(|n| {
                sequence += 1;
                let mut event = work_log::lifecycle::event(
                    &run,
                    "fixture",
                    Some("x".repeat(1000)),
                    "synthetic",
                    now(),
                );
                event.event_id = format!("fixture-{batch}-{n}");
                CommittedRecord {
                    sequence,
                    source_key: format!("fixture-{batch}-{n}"),
                    schema_version: VERSION,
                    record: WorkRecord::Event(event),
                }
            })
            .collect::<Vec<_>>();
        db.apply_batch(&records).unwrap();
    }
    let path = profile.0.join("export.json");
    let started = service
        .work_log_owner(
            "export",
            serde_json::json!({"query":{"taskId":source.id},"path":path,"format":"json"}),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !std::fs::read_dir(&profile.0).unwrap().flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .starts_with(".work-log-export-")
            && e.metadata().is_ok_and(|m| m.len() > 0)
    }) && !path.exists()
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    service.stop_work_log_exports();
    if path.exists() {
        let artifact: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(artifact["format"], "workLogV1");
        assert!(artifact["entries"].is_array());
    }
    assert!(service
        .work_log_owner(
            "exportStatus",
            serde_json::json!({"exportId":started["exportId"]})
        )
        .is_err());
}

#[test]
fn work_log_clear_new_fork_and_handoff_preparation_keep_authoritative_context() {
    let profile = Profile::new();
    let service = Service::open(None, profile.0.clone()).unwrap();
    service.work_log_configure(settings()).unwrap();
    let source = task(&service, "Context fixture");
    let first = message(&source.id, "user", "old source question");
    service
        .mutate(None, |s| {
            s.messages.push(first.clone());
            Ok(())
        })
        .unwrap();
    let old_control = begin(&service, &source);
    let old_run = old_control.current_run_id().unwrap();
    let old_answer = message(&source.id, "assistant", "old source answer");
    service
        .mutate(None, |s| {
            s.messages.push(old_answer.clone());
            s.tasks
                .iter_mut()
                .find(|t| t.id == source.id)
                .unwrap()
                .status = "completed".into();
            Ok(())
        })
        .unwrap();
    service.release_run(&source.id);
    let db = work_log::store::WorkLogDb::open(&profile.0).unwrap();
    flush(&service, &db);
    let old_ref = db
        .connection()
        .unwrap()
        .runs()
        .unwrap()
        .into_iter()
        .find(|r| r.run_id == old_run)
        .unwrap()
        .context_ref;
    service.clear_task_context(&source.id).unwrap();
    let new_question = message(&source.id, "user", "new segment question");
    service
        .mutate(None, |s| {
            s.messages.push(new_question.clone());
            s.tasks
                .iter_mut()
                .find(|t| t.id == source.id)
                .unwrap()
                .native_session_id = Some("missing-fixture-provider-file".into());
            Ok(())
        })
        .unwrap();
    let new_control = begin(&service, &source);
    let new_run = new_control.current_run_id().unwrap();
    let new_answer = message(&source.id, "assistant", "new segment answer");
    service
        .mutate(None, |s| {
            s.messages.push(new_answer.clone());
            s.tasks
                .iter_mut()
                .find(|t| t.id == source.id)
                .unwrap()
                .status = "completed".into();
            Ok(())
        })
        .unwrap();
    service.release_run(&source.id);
    flush(&service, &db);
    let new_ref = db
        .connection()
        .unwrap()
        .runs()
        .unwrap()
        .into_iter()
        .find(|r| r.run_id == new_run)
        .unwrap()
        .context_ref;
    let reader = task(&service, "Context reader");
    let _reader_control = begin(&service, &reader);
    let (installation_id, profile_id) = service.store.work_log_identity().unwrap();
    service
        .work_log_set_grants(vec![OwnerGrant {
            installation_id,
            profile_id,
            caller_task_id: reader.id.clone(),
            operation: Operation::ReadContext,
            scope: GrantScope::Task(source.id.clone()),
            revoked: false,
        }])
        .unwrap();
    let old = service
        .work_log_read_context_protocol(&reader.id, &old_ref, None, Some(64))
        .unwrap();
    let new = service
        .work_log_read_context_protocol(&reader.id, &new_ref, None, Some(64))
        .unwrap();
    assert_eq!(old["messages"][0]["text"], first.text);
    assert_eq!(old["messages"][1]["text"], old_answer.text);
    assert_eq!(new["messages"][0]["text"], new_question.text);
    assert_eq!(new["messages"][1]["text"], new_answer.text);
    assert!(!new.to_string().contains("missing-fixture-provider-file"));
    let independent = task(&service, "New independent chat");
    assert_eq!(
        service.store.work_log_lineage(&independent.id).unwrap().3,
        Some(independent.id.clone())
    );
    let fork = service
        .fork_task(ForkTaskInput {
            source_task_id: source.id.clone(),
            through_message_id: old_answer.id.clone(),
            agent_id: source.agent_id.clone(),
            model_settings: None,
        })
        .unwrap();
    assert_eq!(
        service.store.work_log_lineage(&fork.id).unwrap().3,
        Some(source.id.clone())
    );
    let through = service
        .snapshot()
        .unwrap()
        .messages
        .iter()
        .rev()
        .find(|m| m.task_id == fork.id && m.role == "assistant")
        .unwrap()
        .id
        .clone();
    let grandchild = service
        .fork_task(ForkTaskInput {
            source_task_id: fork.id.clone(),
            through_message_id: through,
            agent_id: source.agent_id.clone(),
            model_settings: None,
        })
        .unwrap();
    assert_eq!(
        service.store.work_log_lineage(&grandchild.id).unwrap().3,
        Some(source.id.clone())
    );
    let mut target_agent = service.snapshot().unwrap().agents[0].clone();
    target_agent.id = id();
    target_agent.name = "Handoff preparation fixture".into();
    service
        .mutate(None, |s| {
            s.agents.push(target_agent.clone());
            Ok(())
        })
        .unwrap();
    let (handoff, brief) = service
        .prepare_handoff(HandoffTaskInput {
            source_task_id: grandchild.id.clone(),
            agent_id: target_agent.id,
            note: None,
        })
        .unwrap();
    assert!(brief.contains("old source answer"));
    assert_eq!(
        service
            .store
            .work_log_lineage(&handoff.id)
            .unwrap()
            .0
            .as_deref(),
        Some("handoff")
    );
    assert_eq!(
        service.store.work_log_lineage(&handoff.id).unwrap().3,
        Some(source.id.clone())
    );
    // Preparation is tested separately from send: no model/provider was invoked.
}
