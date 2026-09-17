//! Deterministic responsiveness invariants, independent of machine speed.
use super::*;
use std::{sync::mpsc, thread, time::Duration};

#[test]
fn native_shortcut_flags_never_wait_for_transcript_state() {
    use std::sync::atomic::Ordering;
    let root = std::env::temp_dir().join(format!("monitter-responsive-keys-{}", id()));
    let service = Service::open(None, root.clone()).unwrap();
    service
        .mutate(None, |snapshot| {
            snapshot.settings.shortcut_mode = menu::VIM_SHORTCUT_MODE.into();
            Ok(())
        })
        .unwrap();
    assert_eq!(service.native_shortcut_mode.load(Ordering::Acquire), 2);
    let state_guard = service.data.lock().unwrap();
    let reader_service = Arc::clone(&service);
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        reader_service.set_native_escape_shield(true);
        tx.send(reader_service.native_escape_shield.load(Ordering::Acquire))
            .unwrap();
    });
    let response = rx.recv_timeout(Duration::from_secs(2));
    drop(state_guard);
    reader.join().unwrap();
    assert!(response.expect("Native shortcut handling waited for transcript state"));
    let rejected: Result<(), String> = service.mutate(None, |snapshot| {
        snapshot.settings.shortcut_mode = menu::STANDARD_SHORTCUT_MODE.into();
        Err("fixture rejection".into())
    });
    assert!(rejected.is_err());
    assert_eq!(service.native_shortcut_mode.load(Ordering::Acquire), 2);
    service
        .mutate(None, |snapshot| {
            snapshot.settings.shortcut_mode = menu::STANDARD_SHORTCUT_MODE.into();
            Ok(())
        })
        .unwrap();
    assert_eq!(service.native_shortcut_mode.load(Ordering::Acquire), 1);
    assert!(!service.native_escape_shield.load(Ordering::Acquire));
    service.set_native_escape_shield(true);
    assert!(!service.native_escape_shield.load(Ordering::Acquire));
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn readers_see_committed_state_without_waiting_for_a_writer() {
    let root = std::env::temp_dir().join(format!("monitter-responsive-read-{}", id()));
    let service = Service::open(None, root.clone()).unwrap();
    let original = service.ui_snapshot(None).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let writer_service = Arc::clone(&service);
    let writer = thread::spawn(move || {
        writer_service.mutate(None, |snapshot| {
            snapshot.settings.theme = "light".into();
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(())
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let reader_service = Arc::clone(&service);
    let (read_tx, read_rx) = mpsc::channel();
    let reader = thread::spawn(move || read_tx.send(reader_service.ui_snapshot(None)).unwrap());
    let while_writing = read_rx.recv_timeout(Duration::from_secs(2));
    // Always release the writer, including on regression, so a failure cannot
    // strand a test worker or its temporary data directory.
    release_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
    reader.join().unwrap();
    let while_writing = while_writing
        .expect("UI read waited for the state writer")
        .unwrap();
    assert_eq!(while_writing.revision, original.revision);
    assert_eq!(while_writing.snapshot, original.snapshot);
    let committed = service.ui_snapshot(None).unwrap();
    assert_ne!(committed.revision, original.revision);
    assert_eq!(committed.snapshot.unwrap().settings.theme, "light");
    drop(service);
    let (_, loaded, _, _) = store::Store::open(root.clone()).unwrap();
    assert_eq!(loaded.settings.theme, "light");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejected_candidate_does_not_publish_or_advance_revision() {
    let root = std::env::temp_dir().join(format!("monitter-responsive-reject-{}", id()));
    let service = Service::open(None, root.clone()).unwrap();
    let original = service.ui_snapshot(None).unwrap();
    let result: Result<(), String> = service.mutate(None, |snapshot| {
        snapshot.settings.theme = "light".into();
        Err("fixture rejection".into())
    });
    assert_eq!(result.unwrap_err(), "fixture rejection");
    let after = service.ui_snapshot(None).unwrap();
    assert_eq!(after.revision, original.revision);
    assert_eq!(after.snapshot, original.snapshot);
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_writers_do_not_lose_updates() {
    let root = std::env::temp_dir().join(format!("monitter-responsive-writes-{}", id()));
    let service = Service::open(None, root.clone()).unwrap();
    let writers = (0..8)
        .map(|index| {
            let service = Arc::clone(&service);
            thread::spawn(move || {
                service.mutate(None, |snapshot| {
                    snapshot.events.push(Arc::new(RunEvent {
                        id: format!("fixture-{index}"),
                        task_id: "fixture".into(),
                        kind: "status".into(),
                        title: "concurrent write".into(),
                        detail: "fixture".into(),
                        created_at: index,
                    }));
                    Ok(())
                })
            })
        })
        .collect::<Vec<_>>();
    for writer in writers {
        writer.join().unwrap().unwrap();
    }
    assert_eq!(service.snapshot().unwrap().events.len(), 8);
    assert_eq!(service.committed_data().unwrap().revision, 8);
    drop(service);
    let (_, loaded, _, _) = store::Store::open(root.clone()).unwrap();
    assert_eq!(loaded.events.len(), 8);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_delivery_failure_keeps_readers_live_during_a_blocked_commit() {
    let root = std::env::temp_dir().join(format!("monitter-responsive-terminal-{}", id()));
    let service = Service::open(None, root.clone()).unwrap();
    service
        .mutate(None, |snapshot| {
            snapshot.tasks.push(
                serde_json::from_value(serde_json::json!({
                    "id":"fixture-task", "agentId":snapshot.agents[0].id, "title":"Fixture",
                    "status":"running", "createdAt":1, "updatedAt":1,
                    "hostId":snapshot.hosts[0].id, "cwd":"/tmp", "provider":"opencode",
                    "model":"", "sandbox":"read-only"
                }))
                .unwrap(),
            );
            Ok(())
        })
        .unwrap();
    let control = runner::RunControl::new(false);
    service
        .runs
        .lock()
        .unwrap()
        .tasks
        .insert("fixture-task".into(), control.clone());
    let database = rusqlite::Connection::open(root.join("state.sqlite3")).unwrap();
    database.execute_batch("BEGIN IMMEDIATE").unwrap();
    let writer_service = service.clone();
    let writer = thread::spawn(move || {
        writer_service.finish_if_current_run(
            "fixture-task",
            &control,
            "fixture delivery failure".into(),
        )
    });
    // finish_if_current_run pins the owner while SQLite waits for our explicit
    // test transaction. Observe that pin before testing the independent reader.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let entered = loop {
        if service.runs.try_lock().is_err() {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };
    let reader_service = service.clone();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        tx.send(reader_service.ui_snapshot(None)).unwrap();
    });
    let while_blocked = rx.recv_timeout(Duration::from_secs(1));
    database.execute_batch("ROLLBACK").unwrap();
    writer.join().unwrap();
    reader.join().unwrap();
    assert!(
        entered,
        "Terminal writer never entered the persistence interval"
    );
    assert!(while_blocked
        .expect("Snapshot reader waited behind a terminal disk write")
        .is_ok());
    assert_eq!(service.snapshot().unwrap().tasks[0].status, "error");
    assert!(!service
        .runs
        .lock()
        .unwrap()
        .tasks
        .contains_key("fixture-task"));
    drop(database);
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}
