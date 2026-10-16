//! Primary-database half of delivery. Called inside the state transaction.
use super::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

pub const DDL: &str = "
 CREATE TABLE IF NOT EXISTS work_log_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS work_log_outbox(
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,source_key TEXT NOT NULL UNIQUE,payload TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS work_log_active(run_id TEXT PRIMARY KEY,task_id TEXT NOT NULL,payload TEXT NOT NULL);
 CREATE INDEX IF NOT EXISTS work_log_active_task ON work_log_active(task_id);
 CREATE TABLE IF NOT EXISTS work_log_context_active(run_id TEXT PRIMARY KEY,payload TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS work_log_loss(id INTEGER PRIMARY KEY CHECK(id=1),gap_id TEXT NOT NULL,started_at INTEGER NOT NULL,through_at INTEGER NOT NULL,dropped INTEGER NOT NULL);
 CREATE TABLE IF NOT EXISTS work_log_receipts(source_key TEXT PRIMARY KEY,digest TEXT NOT NULL,
 sequence INTEGER NOT NULL,accepted_at INTEGER NOT NULL,run_id TEXT NOT NULL);
 CREATE INDEX IF NOT EXISTS work_log_receipts_rate ON work_log_receipts(run_id,accepted_at);
 ";

pub fn settings(c: &Connection) -> Result<CollectionSettings, String> {
    let value: Option<String> = c
        .query_row(
            "SELECT value FROM work_log_meta WHERE key='settings'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    value
        .map(|v| serde_json::from_str(&v).map_err(|_| "work_log_settings_invalid".into()))
        .unwrap_or_else(|| Ok(CollectionSettings::default()))
}
pub fn meta(c: &Connection, key: &str) -> Result<Option<String>, String> {
    c.query_row("SELECT value FROM work_log_meta WHERE key=?1", [key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(|e| e.to_string())
}
pub fn set_meta(c: &Connection, key: &str, value: &str) -> Result<(), String> {
    c.execute("INSERT INTO work_log_meta(key,value)VALUES(?1,?2)ON CONFLICT(key)DO UPDATE SET value=excluded.value",params![key,value]).map(|_|()).map_err(|e|e.to_string())
}
pub fn identity(c: &Connection, key: &str) -> Result<String, String> {
    if let Some(v) = meta(c, key)? {
        return Ok(v);
    }
    let v = crate::model::id();
    set_meta(c, key, &v)?;
    Ok(v)
}
pub fn pending(c: &Connection, limit: usize) -> Result<Vec<CommittedRecord>, String> {
    let mut s = c
        .prepare(
            "SELECT sequence,source_key,payload FROM work_log_outbox ORDER BY sequence LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = s
        .query_map([limit.clamp(1, 100) as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    rows.map(|r| {
        let (sequence, source_key, payload) = r.map_err(|e| e.to_string())?;
        Ok(CommittedRecord {
            sequence,
            source_key,
            schema_version: VERSION,
            record: serde_json::from_str(&payload)
                .map_err(|_| "work_log_outbox_invalid".to_string())?,
        })
    })
    .collect()
}
pub fn active(c: &Connection) -> Result<Vec<WorkRun>, String> {
    let mut s = c
        .prepare("SELECT payload FROM work_log_active")
        .map_err(|e| e.to_string())?;
    let rows = s
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.map(|r| {
        serde_json::from_str(&r.map_err(|e| e.to_string())?)
            .map_err(|_| "work_log_active_invalid".into())
    })
    .collect()
}
pub fn acknowledge(c: &Connection, sequence: i64) -> Result<(), String> {
    c.execute("DELETE FROM work_log_outbox WHERE sequence<=?1", [sequence])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn update_active(tx: &Transaction<'_>, records: &[PendingRecord]) -> Result<(), String> {
    for r in records {
        match &r.record {
            WorkRecord::Context(c) => {
                tx.execute("INSERT INTO work_log_context_active(run_id,payload)VALUES(?1,?2)ON CONFLICT(run_id)DO UPDATE SET payload=excluded.payload",params![c.run_id,serde_json::to_string(c).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
            }
            WorkRecord::Run(run) => {
                if run.outcome.is_some() {
                    tx.execute("DELETE FROM work_log_active WHERE run_id=?1", [&run.run_id])
                        .map_err(|e| e.to_string())?;
                    tx.execute(
                        "DELETE FROM work_log_context_active WHERE run_id=?1",
                        [&run.run_id],
                    )
                    .map_err(|e| e.to_string())?;
                } else {
                    tx.execute("INSERT INTO work_log_active(run_id,task_id,payload)VALUES(?1,?2,?3)ON CONFLICT(run_id)DO UPDATE SET payload=excluded.payload",params![run.run_id,run.task_id,serde_json::to_string(run).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn loss(tx: &Transaction<'_>, dropped: usize) -> Result<(), String> {
    let at = crate::model::now();
    tx.execute("INSERT INTO work_log_loss(id,gap_id,started_at,through_at,dropped)VALUES(1,?1,?2,?2,?3)ON CONFLICT(id)DO UPDATE SET through_at=excluded.through_at,dropped=work_log_loss.dropped+excluded.dropped",params![crate::model::id(),at,dropped as i64]).map_err(|e|e.to_string())?;
    set_meta(tx, "degraded", "outbox_capacity")
}
/// Capacity failure is isolated with a SAVEPOINT. Primary mutation still commits;
/// explicit reports use strict mode and visibly reject instead.
pub fn append(
    tx: &Transaction<'_>,
    records: &[PendingRecord],
    strict: bool,
) -> Result<Vec<i64>, String> {
    if !strict {
        update_active(tx, records)?;
    }
    if !settings(tx)?.enabled {
        return if strict {
            Err("Work log collection is paused.".into())
        } else {
            Ok(vec![])
        };
    }
    let pending_loss: Option<(String, i64, i64, i64)> = tx
        .query_row(
            "SELECT gap_id,started_at,through_at,dropped FROM work_log_loss WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let mut batch = records.to_vec();
    if let Some((id, start, end, count)) = &pending_loss {
        batch.push(PendingRecord {
            source_key: format!("collectionGap:{id}"),
            record: WorkRecord::Gap(CollectorGap {
                gap_id: id.clone(),
                run_id: None,
                started_at: *start,
                ended_at: Some(*end),
                reason: "sourceOutboxCapacity".into(),
                dropped: *count as u64,
            }),
        });
    }
    if batch.is_empty() {
        return Ok(vec![]);
    }
    let payloads = batch
        .iter()
        .map(|r| serde_json::to_string(&r.record).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let (rows, bytes): (i64, i64) = tx
        .query_row(
            "SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM work_log_outbox",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let full = rows as usize + batch.len() > OUTBOX_MAX_ROWS
        || bytes as usize + payloads.iter().map(String::len).sum::<usize>() > OUTBOX_MAX_BYTES
        || payloads.iter().any(|p| p.len() > RECORD_MAX_BYTES);
    if full {
        if !strict && !records.is_empty() {
            loss(tx, records.len())?;
        }
        return if strict {
            Err("Work log outbox is full; report was not accepted.".into())
        } else {
            Ok(vec![])
        };
    }
    tx.execute_batch("SAVEPOINT work_log_append")
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut sequences = Vec::new();
        for (record, payload) in batch.iter().zip(payloads) {
            tx.execute("INSERT INTO work_log_outbox(source_key,payload)VALUES(?1,?2)ON CONFLICT(source_key)DO NOTHING",params![record.source_key,payload]).map_err(|e|e.to_string())?;
            let sequence = tx
                .query_row(
                    "SELECT sequence FROM work_log_outbox WHERE source_key=?1",
                    [&record.source_key],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            sequences.push(sequence);
        }
        Ok::<_, String>(sequences)
    })();
    match result {
        Ok(v) => {
            if pending_loss.is_some() {
                tx.execute("DELETE FROM work_log_loss WHERE id=1", [])
                    .map_err(|e| e.to_string())?;
                set_meta(tx, "degraded", "")?;
            }
            tx.execute_batch("RELEASE work_log_append")
                .map_err(|e| e.to_string())?;
            if strict {
                update_active(tx, records)?;
            }
            Ok(v)
        }
        Err(e) => {
            tx.execute_batch("ROLLBACK TO work_log_append; RELEASE work_log_append")
                .map_err(|e| e.to_string())?;
            if strict {
                Err(e)
            } else {
                loss(tx, records.len())?;
                set_meta(tx, "degraded", "outbox_io")?;
                Ok(vec![])
            }
        }
    }
}
pub fn usage(tx: &Transaction<'_>, sample: &crate::model::RunUsageSample) -> Result<(), String> {
    append(
        tx,
        &[PendingRecord {
            source_key: format!("usage:{}", sample.sample_id),
            record: WorkRecord::Usage(sample.clone()),
        }],
        false,
    )
    .map(|_| ())
}
pub fn report(
    tx: &Transaction<'_>,
    run: &WorkRun,
    input: &ReportInput,
) -> Result<ReportReceipt, String> {
    let payload = serde_json::to_vec(input).map_err(|e| e.to_string())?;
    let digest = format!("{:x}", Sha256::digest(&payload));
    let key = format!(
        "report:{}:{}:{}:{}",
        run.agent_id, run.task_id, run.run_id, input.request_id
    );
    let old: Option<(String, i64, i64)> = tx
        .query_row(
            "SELECT digest,sequence,accepted_at FROM work_log_receipts WHERE source_key=?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((old, sequence, accepted_at)) = old {
        return if old == digest {
            Ok(ReportReceipt {
                source_key: key,
                sequence,
                accepted_at,
            })
        } else {
            Err("Work log request ID was reused with changed content.".into())
        };
    }
    let at = crate::model::now();
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM work_log_receipts WHERE run_id=?1 AND accepted_at>?2",
            params![run.run_id, at - 60_000],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count >= 20 {
        return Err("Work log report rate limit exceeded.".into());
    }
    let event = WorkEvent {
        event_id: crate::model::id(),
        run_id: run.run_id.clone(),
        context_ref: run.context_ref.clone(),
        kind: "report".into(),
        occurred_at: at,
        observed_at: at,
        phase: Some(input.phase.clone()),
        summary: Some(input.summary.clone()),
        plan_id: None,
        plan_item_id: input.plan_item_id.clone(),
        labels: input.labels.clone(),
        source: "agentDeclared".into(),
        coverage: "declared".into(),
    };
    let sequence = append(
        tx,
        &[PendingRecord {
            source_key: key.clone(),
            record: WorkRecord::Event(event),
        }],
        true,
    )?[0];
    tx.execute("INSERT INTO work_log_receipts(source_key,digest,sequence,accepted_at,run_id)VALUES(?1,?2,?3,?4,?5)",params![key,digest,sequence,at,run.run_id]).map_err(|e|e.to_string())?;
    Ok(ReportReceipt {
        source_key: key,
        sequence,
        accepted_at: at,
    })
}
