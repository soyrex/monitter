//! Owner-only local export. The caller validates the destination directory and
//! owns authorization; this module creates one new private file atomically.
use super::{
    query,
    types::{WorkEntry, WorkQuery, WorkSummary},
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportReceipt {
    pub path: PathBuf,
    pub format: String,
    pub entries: u64,
    pub bytes: u64,
    pub resolution: String,
    pub coverage: Vec<String>,
}

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Work log export cancelled.".into())
    } else {
        Ok(())
    }
}

fn create(path: &Path) -> Result<File, String> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    opts.open(path)
        .map_err(|_| "Work log export destination unavailable.".into())
}

fn csv_cell(value: &str) -> String {
    // Spreadsheet importers may ignore leading ASCII space or control bytes.
    // Prefix a literal apostrophe before the first dangerous character, even
    // when it follows whitespace. Quote every cell to preserve exact columns.
    let dangerous = value.trim_start_matches(|c: char| c.is_whitespace() || c.is_control());
    let protect = dangerous
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@'));
    let value = if protect {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn csv_row(writer: &mut impl Write, values: &[String]) -> Result<(), String> {
    let line = values
        .iter()
        .map(|v| csv_cell(v))
        .collect::<Vec<_>>()
        .join(",");
    writer
        .write_all(line.as_bytes())
        .and_then(|_| writer.write_all(b"\r\n"))
        .map_err(|_| "Work log export write failed.".into())
}
fn entry_row(e: &WorkEntry) -> Vec<String> {
    vec![
        "event".into(),
        e.sequence.to_string(),
        e.source_key.clone(),
        e.run.run_id.clone(),
        e.run.task_id.clone(),
        e.run.project_id.clone().unwrap_or_default(),
        e.event.occurred_at.to_string(),
        e.event.kind.clone(),
        e.event.phase.clone().unwrap_or_default(),
        e.event.summary.clone().unwrap_or_default(),
        e.event.coverage.clone(),
        String::new(),
        String::new(),
    ]
}
fn csv_header() -> Vec<String> {
    [
        "recordType",
        "sequence",
        "sourceKey",
        "runId",
        "taskId",
        "projectId",
        "occurredAt",
        "eventKind",
        "phase",
        "summary",
        "coverage",
        "resolution",
        "missingUsageRuns",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Exports one read-only SQLite snapshot. A failed or cancelled export removes
/// its newly-created file; pre-existing files are never opened for writing.
fn export_with_hook(
    conn: &Connection,
    query: &WorkQuery,
    path: &Path,
    format: &str,
    cancel: &AtomicBool,
    on_page: impl Fn(u64),
) -> Result<ExportReceipt, String> {
    if !matches!(format, "json" | "csv") {
        return Err("Unsupported work log export format.".into());
    }
    if query.cursor.is_some() {
        return Err("Export does not accept a page cursor.".into());
    }
    if query.from.is_none()
        && query.through.is_none()
        && query.project_id.is_none()
        && query.task_id.is_none()
    {
        return Err("Work log export requires a range or source filter.".into());
    }
    check(cancel)?;
    if fs::symlink_metadata(path).is_ok() {
        return Err("Work log export destination already exists.".into());
    }
    let temporary = path
        .parent()
        .ok_or("Invalid export destination.")?
        .join(format!(".work-log-export-{}.partial", uuid::Uuid::new_v4()));
    let file = create(&temporary)?;
    let result = (|| -> Result<ExportReceipt, String> {
        conn.execute_batch("BEGIN DEFERRED")
            .map_err(|_| "Work log export unavailable.")?;
        let snapshot = (|| -> Result<(u64, WorkSummary), String> {
            let summary = if query.phase.is_some() {
                // A phase filters events; run-level duration, resources and
                // usage cannot be divided among those events honestly.
                serde_json::from_value(serde_json::json!({
                    "days":[],"runs":0,"agentElapsedMs":0,"projectCoveredMs":0,
                    "knownWaitMs":null,"runnableMs":null,"cpuMs":null,
                    "rssByteMs":null,"coveredResourceMs":0,
                    "sampledPeakRssBytes":null,"tokens":crate::model::UsageTokens::default(),
                    "missingUsageRuns":0,"reportedCostUnits":null,"costScale":1000000,
                    "currency":null,"coverage":["phaseTimelineOnly","metricsUnallocated"],
                    "resolution":"eventOnly"
                }))
                .map_err(|_| "Work log export unavailable.")?
            } else {
                query::summary(conn, query, None)?
            };
            let mut writer = BufWriter::new(file);
            if format == "json" {
                writer
                    .write_all(b"{\"format\":\"workLogV1\",\"query\":")
                    .map_err(|_| "Work log export write failed.")?;
                serde_json::to_writer(&mut writer, query)
                    .map_err(|_| "Work log export write failed.")?;
                writer
                    .write_all(b",\"summary\":")
                    .map_err(|_| "Work log export write failed.")?;
                serde_json::to_writer(&mut writer, &summary)
                    .map_err(|_| "Work log export write failed.")?;
                writer
                    .write_all(b",\"entries\":[")
                    .map_err(|_| "Work log export write failed.")?;
            } else {
                csv_row(&mut writer, &csv_header())?;
                csv_row(
                    &mut writer,
                    &[
                        "summary".into(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        serde_json::to_string(&summary)
                            .map_err(|_| "Work log export write failed.")?,
                        summary.coverage.join(";"),
                        summary.resolution.clone(),
                        summary.missing_usage_runs.to_string(),
                    ],
                )?;
            }
            let mut q = query.clone();
            q.cursor = None;
            q.limit = Some(64);
            let mut count = 0u64;
            loop {
                check(cancel)?;
                let page = query::page(conn, &q, None)?;
                for e in &page.entries {
                    check(cancel)?;
                    if format == "json" {
                        if count > 0 {
                            writer
                                .write_all(b",")
                                .map_err(|_| "Work log export write failed.")?;
                        }
                        serde_json::to_writer(&mut writer, e)
                            .map_err(|_| "Work log export write failed.")?;
                    } else {
                        csv_row(&mut writer, &entry_row(e))?;
                    }
                    count = count.checked_add(1).ok_or("Work log export too large.")?;
                }
                on_page(count);
                check(cancel)?;
                match page.next_cursor {
                    Some(cursor) => q.cursor = Some(cursor),
                    None => break,
                }
            }
            if format == "json" {
                writer
                    .write_all(b"]}\n")
                    .map_err(|_| "Work log export write failed.")?;
            }
            writer
                .flush()
                .map_err(|_| "Work log export write failed.")?;
            writer
                .get_ref()
                .sync_all()
                .map_err(|_| "Work log export write failed.")?;
            Ok((count, summary))
        })();
        let end = if snapshot.is_ok() {
            "COMMIT"
        } else {
            "ROLLBACK"
        };
        conn.execute_batch(end)
            .map_err(|_| "Work log export unavailable.")?;
        let (entries, summary) = snapshot?;
        check(cancel)?;
        let bytes = fs::metadata(&temporary)
            .map_err(|_| "Work log export unavailable.")?
            .len();
        Ok(ExportReceipt {
            path: path.to_path_buf(),
            format: format.into(),
            entries,
            bytes,
            resolution: summary.resolution,
            coverage: summary.coverage,
        })
    })();
    let result = result.and_then(|receipt| {
        check(cancel)?;
        // Same-directory hard-link publication is atomic and refuses an
        // existing destination. A stopped process can leave only a private
        // hidden partial, never a truncated file at the requested final path.
        fs::hard_link(&temporary, path)
            .map_err(|_| "Work log export publication failed.".to_string())?;
        if let Some(parent) = path.parent() {
            if let Ok(directory) = File::open(parent) {
                let _ = directory.sync_all();
            }
        }
        Ok(receipt)
    });
    let _ = fs::remove_file(&temporary);
    result
}

pub fn export(
    conn: &Connection,
    query: &WorkQuery,
    path: &Path,
    format: &str,
    cancel: &AtomicBool,
) -> Result<ExportReceipt, String> {
    export_with_hook(conn, query, path, format, cancel, |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[test]
    fn formula_prefixes_after_whitespace() {
        assert_eq!(csv_cell(" =SUM(A1)"), "\"' =SUM(A1)\"");
        assert_eq!(csv_cell("\t@evil"), "\"'\t@evil\"");
        assert_eq!(csv_cell("plain\"text"), "\"plain\"\"text\"");
    }
    #[test]
    fn cancellation_creates_no_partial_file_and_existing_file_is_preserved() {
        let c = Connection::open_in_memory().unwrap();
        let path = std::env::temp_dir().join(format!(
            "work-log-export-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let q = WorkQuery {
            from: Some(0),
            through: Some(1000),
            ..Default::default()
        };
        assert!(export(&c, &q, &path, "json", &AtomicBool::new(true)).is_err());
        assert!(!path.exists());
        fs::write(&path, b"existing").unwrap();
        assert!(export(&c, &q, &path, "json", &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing");
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn json_and_csv_exports_complete_one_snapshot() {
        let c = query::tests::db();
        query::tests::insert(&c, "a", "task", "p", "task", 0, 1000, 1);
        let q = WorkQuery {
            from: Some(0),
            through: Some(2000),
            ..Default::default()
        };
        for format in ["json", "csv"] {
            let path = std::env::temp_dir().join(format!(
                "work-log-export-test-{}.{}",
                uuid::Uuid::new_v4(),
                format
            ));
            let receipt = export(&c, &q, &path, format, &AtomicBool::new(false)).unwrap();
            assert_eq!(receipt.entries, 1);
            let bytes = fs::read(&path).unwrap();
            assert_eq!(receipt.bytes, bytes.len() as u64);
            if format == "json" {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["entries"].as_array().unwrap().len(), 1);
                assert_eq!(value["summary"]["runs"], 1);
            } else {
                let text = String::from_utf8(bytes).unwrap();
                assert!(text.contains("recordType"));
                assert!(text.contains("\"event\""));
                assert!(text.contains("rawOrBucket"));
            }
            fs::remove_file(path).unwrap();
        }
    }
    #[test]
    fn cancellation_after_first_page_removes_created_file() {
        let c = query::tests::db();
        query::tests::insert(&c, "a", "task", "p", "task", 0, 1000, 1);
        for i in 2..=66 {
            let event_id = format!("event-{i}");
            let source = format!("source-{i}");
            c.execute("INSERT INTO events SELECT ?1,run_id,phase,occurred_at,?2,?3,json_set(raw_json,'$.eventId',?1) FROM events WHERE event_id='event-a'",
                rusqlite::params![event_id,i,source]).unwrap();
        }
        let q = WorkQuery {
            from: Some(0),
            through: Some(2000),
            ..Default::default()
        };
        assert!(query::page(
            &c,
            &WorkQuery {
                limit: Some(64),
                ..q.clone()
            },
            None
        )
        .unwrap()
        .next_cursor
        .is_some());
        let path = std::env::temp_dir().join(format!(
            "work-log-export-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let cancel = AtomicBool::new(false);
        let result = export_with_hook(&c, &q, &path, "json", &cancel, |entries| {
            if entries > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(result.is_err());
        assert!(!path.exists());
    }
    #[test]
    fn phase_export_has_unallocated_summary_and_filtered_events() {
        let c = query::tests::db();
        query::tests::insert(&c, "a", "task", "p", "task", 0, 1000, 1);
        let q = WorkQuery {
            from: Some(0),
            through: Some(2000),
            phase: Some("plan".into()),
            ..Default::default()
        };
        let path = std::env::temp_dir().join(format!(
            "work-log-export-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let receipt = export(&c, &q, &path, "json", &AtomicBool::new(false)).unwrap();
        assert_eq!(receipt.entries, 1);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(value["summary"]["coverage"]
            .to_string()
            .contains("metricsUnallocated"));
        fs::remove_file(path).unwrap();
    }
}
