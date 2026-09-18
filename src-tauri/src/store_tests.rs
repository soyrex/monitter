use super::*;
use crate::model::{id, RunEvent, SubagentSession, SubagentTranscriptEntry};
use std::{fs, process::Command, sync::Arc};

const CRASH_RECOVERY_DIRECTORY: &str = "MONITTER_STORE_CRASH_RECOVERY_DIRECTORY";

#[test]
fn save_is_private_and_preserves_task_hosts() {
    let dir = directory();
    let (store, snapshot, mut task_hosts, mut attachments) = Store::open(dir.clone()).unwrap();
    task_hosts.insert("task".into(), snapshot.hosts[0].clone());
    attachments.insert(
        "attachment".into(),
        StoredAttachment {
            attachment: crate::attachments::Attachment {
                id: "attachment".into(),
                name: "x.txt".into(),
                mime_type: "text/plain".into(),
                size: 1,
                path: "/tmp/x".into(),
                preview_data_url: None,
                source_id: None,
            },
            host_id: snapshot.hosts[0].id.clone(),
            cwd: "/tmp".into(),
        },
    );
    store.save(&snapshot, &task_hosts, &attachments).unwrap();
    drop(store);
    let (reopened, _, loaded, loaded_attachments) = Store::open(dir.clone()).unwrap();
    assert_eq!(loaded.get("task"), task_hosts.get("task"));
    assert_eq!(loaded_attachments, attachments);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(dir.join("state.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

fn directory() -> PathBuf {
    std::env::temp_dir().join(format!("monitter-store-test-{}", id()))
}
fn event(id: &str) -> Arc<RunEvent> {
    Arc::new(RunEvent {
        id: id.into(),
        task_id: "task".into(),
        kind: "log".into(),
        title: id.into(),
        detail: Arc::from("detail"),
        created_at: 1,
    })
}
fn usage(id: &str, classification: &str, total: i64) -> RunUsageSample {
    serde_json::from_value(serde_json::json!({
        "sampleId": id, "runId": "run", "taskId": "task", "provider": "codex",
        "startedAt": 1, "observedAt": 2, "finalSample": false,
        "classification": classification, "tokens": {"total": total}
    }))
    .unwrap()
}

fn subagent_session(id: &str) -> SubagentSession {
    SubagentSession {
        id: id.into(),
        source: "acp".into(),
        parent_task_id: "task".into(),
        parent_thread_id: None,
        collaboration_id: None,
        agent_path: Some("root/research".into()),
        agent_thread_id: None,
        prompt: Some("Inspect the bounded fixture.".into()),
        model: Some("test-model".into()),
        reasoning_effort: None,
        status: "running".into(),
        result: None,
        error: None,
        created_at: 10,
        updated_at: 10,
    }
}

fn subagent_entry(id: &str, text: &str, created_at: i64) -> SubagentTranscriptEntry {
    SubagentTranscriptEntry {
        id: id.into(),
        role: "assistant".into(),
        text: text.into(),
        created_at,
    }
}

#[test]
fn exclusive_owner_releases_lease_on_drop() {
    let path = directory();
    let (store, _, _, _) = Store::open(path.clone()).unwrap();
    assert!(Store::open(path.clone()).is_err());
    drop(store);
    drop(Store::open(path.clone()).unwrap());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn event_edits_deletes_and_reordering_survive_reopen() {
    let path = directory();
    let (store, mut snapshot, hosts, attachments) = Store::open(path.clone()).unwrap();
    snapshot.events = vec![event("one"), event("two"), event("three")];
    store.save(&snapshot, &hosts, &attachments).unwrap();
    let before = snapshot.clone();
    snapshot.events = vec![event("three"), event("one")];
    Arc::make_mut(&mut snapshot.events[0]).detail = Arc::from("changed");
    store
        .save_update(
            (&before, &hosts, &attachments),
            (&snapshot, &hosts, &attachments),
        )
        .unwrap();
    drop(store);
    let (store, reopened, _, _) = Store::open(path.clone()).unwrap();
    assert_eq!(reopened.events, snapshot.events);
    drop(store);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn subagent_projection_round_trips_updates_and_deletes_atomically() {
    let path = directory();
    let (store, mut snapshot, hosts, attachments) = Store::open(path.clone()).unwrap();
    snapshot.subagent_sessions = vec![subagent_session("child")];
    snapshot.subagent_transcripts.insert(
        "child".into(),
        vec![
            subagent_entry("entry-1", "small captured detail", 11),
            subagent_entry("entry-2", "still deliberately bounded", 12),
        ],
    );
    store.save(&snapshot, &hosts, &attachments).unwrap();

    let before = snapshot.clone();
    snapshot.subagent_sessions[0].status = "completed".into();
    snapshot.subagent_sessions[0].result = Some("done".into());
    snapshot.subagent_sessions[0].updated_at = 13;
    snapshot
        .subagent_transcripts
        .get_mut("child")
        .unwrap()
        .push(subagent_entry("entry-3", "final bounded detail", 13));
    store
        .save_update(
            (&before, &hosts, &attachments),
            (&snapshot, &hosts, &attachments),
        )
        .unwrap();
    drop(store);

    let (store, reopened, reopened_hosts, reopened_attachments) =
        Store::open(path.clone()).unwrap();
    assert_eq!(reopened.subagent_sessions, snapshot.subagent_sessions);
    assert_eq!(reopened.subagent_transcripts, snapshot.subagent_transcripts);

    let mut deleted = reopened.clone();
    deleted.subagent_sessions.clear();
    deleted.subagent_transcripts.clear();
    store
        .save_update(
            (&reopened, &reopened_hosts, &reopened_attachments),
            (&deleted, &reopened_hosts, &reopened_attachments),
        )
        .unwrap();
    drop(store);

    let (store, reopened, _, _) = Store::open(path.clone()).unwrap();
    assert!(reopened.subagent_sessions.is_empty());
    assert!(reopened.subagent_transcripts.is_empty());
    drop(store);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn usage_is_staged_deduplicated_atomic_and_removable() {
    let path = directory();
    let (store, snapshot, hosts, attachments) = Store::open(path.clone()).unwrap();
    store.stage_usage(usage("a", "delta", 10)).unwrap();
    store.stage_usage(usage("a", "delta", 10)).unwrap();
    store.stage_usage(usage("b", "cumulative", 50)).unwrap();
    store.stage_usage(usage("c", "delta", 5)).unwrap();
    assert_eq!(
        store.usage_overview().unwrap().recent_runs[0].tokens.total,
        Some(55)
    );
    store.save(&snapshot, &hosts, &attachments).unwrap();
    drop(store);
    let (store, snapshot, hosts, attachments) = Store::open(path.clone()).unwrap();
    assert_eq!(
        store.usage_overview().unwrap().recent_runs[0].tokens.total,
        Some(55)
    );
    store.remove_task_usage("task").unwrap();
    store.save(&snapshot, &hosts, &attachments).unwrap();
    drop(store);
    let (store, _, _, _) = Store::open(path.clone()).unwrap();
    assert!(store.usage_overview().unwrap().recent_runs.is_empty());
    drop(store);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn rejected_transaction_preserves_state_and_requires_reopen() {
    let path = directory();
    let (store, before, hosts, attachments) = Store::open(path.clone()).unwrap();
    let connection = rusqlite::Connection::open(path.join(migration::DATABASE_NAME)).unwrap();
    connection.execute_batch("CREATE TRIGGER fixture_fail BEFORE INSERT ON entities WHEN NEW.collection='events' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    let mut after = before.clone();
    after.events.push(event("failed"));
    store
        .stage_usage(usage("uncommitted", "delta", 10))
        .unwrap();
    assert!(store
        .save_update(
            (&before, &hosts, &attachments),
            (&after, &hosts, &attachments)
        )
        .is_err());
    assert!(store
        .save(&before, &hosts, &attachments)
        .unwrap_err()
        .contains("reopen"));
    assert_eq!(store.database.read_snapshot().unwrap().0, before);
    assert!(store.database.read_snapshot().unwrap().3.is_empty());
    drop(store);
    connection
        .execute_batch("DROP TRIGGER fixture_fail")
        .unwrap();
    drop(connection);
    let (store, reopened, _, _) = Store::open(path.clone()).unwrap();
    assert_eq!(reopened, before);
    assert!(store.usage_overview().unwrap().recent_runs.is_empty());
    drop(store);
    fs::remove_dir_all(path).unwrap();
}

/// This is invoked only by `crash_recovery_replays_committed_wal_after_process_exit`.
/// Keeping the helper itself inert in ordinary test runs avoids touching any
/// profile unless its parent supplies an exact, freshly-created temporary path.
#[test]
fn store_crash_recovery_child() {
    let Ok(directory) = std::env::var(CRASH_RECOVERY_DIRECTORY) else {
        return;
    };
    let directory = PathBuf::from(directory);
    let (store, mut snapshot, hosts, attachments) = Store::open(directory.clone()).unwrap();
    let before = snapshot.clone();
    snapshot.events.push(event("committed-before-crash"));
    store
        .stage_usage(usage("committed-usage-before-crash", "delta", 17))
        .unwrap();
    store
        .save_update(
            (&before, &hosts, &attachments),
            (&snapshot, &hosts, &attachments),
        )
        .unwrap();

    // Leave an explicit write transaction open. process::exit skips Drop for
    // both connections and the Store lease, reproducing an abrupt process
    // death rather than a normal close/checkpoint.
    let connection = rusqlite::Connection::open(directory.join(migration::DATABASE_NAME)).unwrap();
    connection
        .execute_batch(
            "BEGIN IMMEDIATE;
             INSERT INTO entities(collection,id,position,task_id,created_at,payload)
             VALUES('crash-recovery-fixture','uncommitted',0,NULL,NULL,'{}');",
        )
        .unwrap();
    std::process::exit(0);
}

#[test]
fn crash_recovery_replays_committed_wal_after_process_exit() {
    let path = directory();
    let (store, _, _, _) = Store::open(path.clone()).unwrap();
    drop(store);

    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("store::tests::store_crash_recovery_child")
        .arg("--nocapture")
        .env(CRASH_RECOVERY_DIRECTORY, &path)
        .status()
        .unwrap();
    assert!(status.success());

    let wal = path.join(format!("{}-wal", migration::DATABASE_NAME));
    assert!(
        fs::metadata(&wal).unwrap().len() > 0,
        "the child should leave committed data in the WAL"
    );

    // The operating system releases the child lease and SQLite rolls back its
    // unfinished transaction while replaying the committed WAL frames.
    let (store, snapshot, _, _) = Store::open(path.clone()).unwrap();
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.id == "committed-before-crash"));
    assert_eq!(
        store.usage_overview().unwrap().recent_runs[0].tokens.total,
        Some(17)
    );
    let connection = rusqlite::Connection::open(path.join(migration::DATABASE_NAME)).unwrap();
    let uncommitted: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE collection='crash-recovery-fixture'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(uncommitted, 0);
    drop(connection);
    drop(store);
    fs::remove_dir_all(path).unwrap();
}
