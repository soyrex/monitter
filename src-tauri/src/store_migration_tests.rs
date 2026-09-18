use super::*;
use std::{fs, path::PathBuf};

fn dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("monitter-migration-{label}-{}", id()));
    private_dir(&path).unwrap();
    path
}
fn clean(path: PathBuf) {
    let _ = fs::remove_dir_all(path);
}
fn intent(generation: String, sources: Vec<Source>, hash: String) -> Intent {
    Intent {
        version: 1,
        generation,
        sources,
        state_sha256: hash,
    }
}

#[test]
fn fresh_open_installs_database_and_guard() {
    let path = dir("fresh");
    let db = open_database(&path).unwrap();
    drop(db);
    assert!(path.join(DATABASE_NAME).is_file());
    assert!(guard_present(&path).unwrap());
    clean(path);
}

#[test]
fn corrupt_legacy_and_missing_committed_prefix_do_not_install() {
    let path = dir("corrupt");
    let state = path.join("state.json");
    fs::write(&state, b"not-json").unwrap();
    let before = fs::read(&state).unwrap();
    assert!(open_database(&path).is_err());
    assert_eq!(fs::read(&state).unwrap(), before);
    assert!(!path.join(DATABASE_NAME).exists());
    clean(path);
}

#[test]
fn guarded_missing_or_corrupt_database_never_falls_back() {
    let path = dir("guard");
    write_guard(&path).unwrap();
    assert!(open_database(&path).is_err());
    fs::write(path.join(DATABASE_NAME), b"corrupt").unwrap();
    assert!(open_database(&path).is_err());
    clean(path);
}

#[test]
fn invalid_intent_names_and_uuid_are_rejected() {
    let path = dir("intent");
    let bad = intent("not-a-uuid".into(), vec![], "0".repeat(64));
    fs::write(path.join(INTENT_NAME), serde_json::to_vec(&bad).unwrap()).unwrap();
    assert!(open_database(&path).is_err());
    let bad = intent(
        id(),
        vec![Source {
            name: "../state.json".into(),
            sha256: "0".repeat(64),
        }],
        "0".repeat(64),
    );
    fs::write(path.join(INTENT_NAME), serde_json::to_vec(&bad).unwrap()).unwrap();
    assert!(open_database(&path).is_err());
    clean(path);
}

#[test]
fn partial_temp_database_is_quarantined_then_fresh_state_rebuilds() {
    let path = dir("partial");
    let generation = id();
    let state = legacy_store::read(&path).unwrap().0;
    let manifest = intent(generation.clone(), vec![], state_hash(&state).unwrap());
    fs::write(
        path.join(INTENT_NAME),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        path.join(format!("sqlite-import-{generation}.sqlite3")),
        b"partial",
    )
    .unwrap();
    let db = open_database(&path).unwrap();
    drop(db);
    assert!(path.join(DATABASE_NAME).exists());
    assert!(fs::read_dir(&path).unwrap().flatten().any(|entry| entry
        .file_name()
        .to_string_lossy()
        .starts_with("sqlite-import-abandoned-")));
    clean(path);
}

#[test]
fn embedded_legacy_is_guarded_and_backed_up_exactly() {
    let path = dir("legacy");
    let state = path.join("state.json");
    let bytes = serde_json::to_vec(&crate::model::default_snapshot()).unwrap();
    fs::write(&state, &bytes).unwrap();
    let db = open_database(&path).unwrap();
    drop(db);
    assert!(guard_present(&path).unwrap());
    let backup = fs::read_dir(&path)
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("legacy-before-sqlite-")
        })
        .unwrap()
        .path();
    assert_eq!(fs::read(backup.join("state.json")).unwrap(), bytes);
    clean(path);
}

#[test]
fn old_snapshot_guard_is_rejected_without_database_fallback() {
    let path = dir("old-guard");
    fs::write(
        path.join("state.json"),
        br#"{"events":"monitter-sqlite-v1","database":"state.sqlite3","schemaVersion":0}"#,
    )
    .unwrap();
    assert!(open_database(&path).is_err());
    assert!(!path.join(DATABASE_NAME).exists());
    clean(path);
}

#[test]
fn installed_guard_is_not_a_legacy_snapshot() {
    let path = dir("guard-snapshot");
    let db = open_database(&path).unwrap();
    drop(db);
    assert!(serde_json::from_slice::<crate::model::Snapshot>(
        &fs::read(path.join("state.json")).unwrap()
    )
    .is_err());
    clean(path);
}

#[test]
fn missing_or_truncated_committed_journal_and_usage_prefixes_fail_closed() {
    for (journal, usage) in [(true, false), (false, true)] {
        let path = dir("prefix");
        let generation = id();
        let mut value = serde_json::to_value(crate::model::default_snapshot()).unwrap();
        if journal {
            value["_eventJournal"] =
                serde_json::json!({"generation":generation,"committedCount":1});
        }
        if usage {
            value["_usageLedger"] = serde_json::json!({"committedCount":1,"capturedSince":0});
        }
        fs::write(path.join("state.json"), serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(open_database(&path).is_err());
        assert!(!path.join(DATABASE_NAME).exists());
        let prefix_path = if journal {
            path.join(format!("events-{generation}.jsonl"))
        } else {
            path.join("usage-v1.jsonl")
        };
        fs::write(&prefix_path, b"").unwrap();
        assert!(open_database(&path).is_err());
        fs::write(&prefix_path, b"{\"incomplete\":").unwrap();
        assert!(open_database(&path).is_err());
        assert!(!path.join(DATABASE_NAME).exists());
        clean(path);
    }
}

#[test]
fn changed_legacy_after_intent_fails_and_preserves_source() {
    let path = dir("changed");
    let state = path.join("state.json");
    fs::write(
        &state,
        serde_json::to_vec(&crate::model::default_snapshot()).unwrap(),
    )
    .unwrap();
    let imported = legacy_store::read(&path).unwrap().0;
    let manifest = intent(
        id(),
        vec![Source {
            name: "state.json".into(),
            sha256: file_hash(&state).unwrap(),
        }],
        state_hash(&imported).unwrap(),
    );
    fs::write(
        path.join(INTENT_NAME),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(&state, b"{}\n").unwrap();
    let before = fs::read(&state).unwrap();
    assert!(open_database(&path).is_err());
    assert_eq!(fs::read(&state).unwrap(), before);
    assert!(!path.join(DATABASE_NAME).exists());
    clean(path);
}

#[test]
fn journal_and_usage_import_only_committed_prefix_and_keep_exact_backups() {
    let path = dir("prefix-valid");
    let generation = id();
    let event = serde_json::json!({"id":"event", "taskId":"task", "kind":"log", "title":"title", "detail":"detail", "createdAt":1});
    let usage = serde_json::json!({"sampleId":"sample", "runId":"run", "taskId":"task", "provider":"codex", "startedAt":1, "observedAt":2, "finalSample":false, "classification":"delta", "tokens":{"total":12}});
    let mut value = serde_json::to_value(crate::model::default_snapshot()).unwrap();
    value["_eventJournal"] = serde_json::json!({"generation":generation,"committedCount":1});
    value["_usageLedger"] = serde_json::json!({"committedCount":1,"capturedSince":42});
    let sources = [
        (
            "state.json".to_string(),
            serde_json::to_vec(&value).unwrap(),
        ),
        (
            format!("events-{generation}.jsonl"),
            format!("{event}\n{{unfinished tail").into_bytes(),
        ),
        (
            "usage-v1.jsonl".to_string(),
            format!("{usage}\n{{unfinished tail").into_bytes(),
        ),
    ];
    for (name, bytes) in &sources {
        fs::write(path.join(name), bytes).unwrap();
    }
    let db = open_database(&path).unwrap();
    let loaded = db.read_snapshot().unwrap();
    assert_eq!(loaded.0.events.len(), 1);
    assert_eq!(loaded.0.events[0].id, "event");
    assert_eq!(loaded.3.len(), 1);
    assert_eq!(loaded.3[0].tokens.total, Some(12));
    assert_eq!(loaded.4, Some(42));
    let backup = fs::read_dir(&path)
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("legacy-before-sqlite-")
        })
        .unwrap()
        .path();
    for (name, bytes) in sources {
        assert_eq!(fs::read(backup.join(name)).unwrap(), bytes);
    }
    drop(db);
    clean(path);
}

#[test]
fn installed_database_before_guard_resumes_verified_migration() {
    let path = dir("installed-before-guard");
    let bytes = serde_json::to_vec(&crate::model::default_snapshot()).unwrap();
    fs::write(path.join("state.json"), &bytes).unwrap();
    let imported = legacy_store::read(&path).unwrap().0;
    let manifest = intent(
        id(),
        vec![Source {
            name: "state.json".into(),
            sha256: file_hash(&path.join("state.json")).unwrap(),
        }],
        state_hash(&imported).unwrap(),
    );
    fs::write(
        path.join(INTENT_NAME),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let db = Database::create(&path.join(DATABASE_NAME)).unwrap();
    db.replace_all(
        (&imported.0, &imported.1, &imported.2),
        &imported.3,
        imported.4,
    )
    .unwrap();
    db.checkpoint().unwrap();
    drop(db);
    let reopened = open_database(&path).unwrap();
    assert_eq!(
        state_hash(&reopened.read_snapshot().unwrap()).unwrap(),
        manifest.state_sha256
    );
    assert!(guard_present(&path).unwrap());
    assert!(!path.join(INTENT_NAME).exists());
    assert_eq!(
        fs::read(
            path.join(format!("legacy-before-sqlite-{}", manifest.generation))
                .join("state.json")
        )
        .unwrap(),
        bytes
    );
    drop(reopened);
    clean(path);
}

#[cfg(unix)]
#[test]
fn legacy_symlinks_are_rejected_before_import() {
    use std::os::unix::fs::symlink;
    let path = dir("symlink");
    let target = path.join("original.json");
    let bytes = serde_json::to_vec(&crate::model::default_snapshot()).unwrap();
    fs::write(&target, &bytes).unwrap();
    symlink(&target, path.join("state.json")).unwrap();
    assert!(legacy_store::read(&path).is_err());
    assert!(open_database(&path).is_err());
    assert_eq!(fs::read(target).unwrap(), bytes);
    assert!(!path.join(DATABASE_NAME).exists());
    clean(path);
}
