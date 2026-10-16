//! Opt-in, ignored Work Log performance fixtures.
//!
//! These are measurements, not budget assertions. The `ten_minute_collector`
//! test deliberately takes ten minutes and is never run by the normal suite.
//!
//! Run after `work_log::performance` is registered from the test module:
//! `cargo test --manifest-path src-tauri/Cargo.toml work_log::performance -- --ignored --nocapture`
//!
//! A disposable temporary directory is used by default. To retain a corpus,
//! set `WORK_LOG_BENCH_DIR` to a path beneath a directory named `verification`;
//! a unique child is created and is never recursively removed in that mode.

#[cfg(test)]
mod tests {
    use crate::work_log::{
        outbox, query, resources::ResourceTracker, sampler::LocalProcessSource, schema,
        store::WorkLogDb, types::ProcessIdentity, types::ProcessObservation, types::ProcessSource,
        types::RunRoot, CollectionSettings, CommittedRecord, PendingRecord, ResourceSample,
        WorkEvent, WorkQuery, WorkRecord, WorkRun, VERSION,
    };
    use rusqlite::{params, Connection, Error as SqliteError, ErrorCode};
    use serde_json::json;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    const MONTH_START: i64 = 1_790_000_000_000;

    struct BenchDir {
        path: PathBuf,
        remove_on_drop: bool,
    }
    impl BenchDir {
        fn new(label: &str) -> Self {
            let id = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            if let Some(root) = std::env::var_os("WORK_LOG_BENCH_DIR") {
                let root = PathBuf::from(root);
                let verified = root.components().any(|c| c.as_os_str() == "verification")
                    && !root
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir));
                assert!(verified, "WORK_LOG_BENCH_DIR must be beneath verification");
                fs::create_dir_all(&root).expect("create benchmark root");
                let path = root.join(format!("work-log-{label}-{id}"));
                fs::create_dir(&path).expect("create unique benchmark child");
                Self {
                    path,
                    remove_on_drop: false,
                }
            } else {
                let path = std::env::temp_dir().join(format!("work-log-{label}-{id}"));
                fs::create_dir(&path).expect("create temporary benchmark directory");
                Self {
                    path,
                    remove_on_drop: true,
                }
            }
        }
    }
    impl Drop for BenchDir {
        fn drop(&mut self) {
            if self.remove_on_drop {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }

    fn run(id: usize) -> WorkRun {
        WorkRun {
            run_id: format!("bench-run-{id}"),
            task_id: format!("bench-task-{id}"),
            project_id: Some("bench-project".into()),
            agent_id: "synthetic-agent".into(),
            host_id: "synthetic-host".into(),
            backend_session_id: format!("session-{id}"),
            parent_run_id: None,
            parent_task_id: None,
            context_ref: format!("context-{id}"),
            title: "synthetic benchmark fixture".into(),
            project_name: Some("bench".into()),
            agent_name: "fixture".into(),
            host_name: "local-fixture".into(),
            harness: "fixture".into(),
            configured_model: Some("synthetic-only".into()),
            observed_model: None,
            cwd: None,
            repository_root: None,
            commit_sha: None,
            adapter_version: None,
            reasoning_setting: None,
            trigger: "manual".into(),
            remote: false,
            started_at: MONTH_START,
            observed_at: MONTH_START + 1,
            ended_at: None,
            elapsed_ms: None,
            outcome: None,
            coverage: "synthetic".into(),
        }
    }

    fn closed_run(id: usize) -> WorkRun {
        let mut value = run(id);
        value.ended_at = Some(MONTH_START + 2_592_000_000);
        value.observed_at = value.ended_at.unwrap();
        value.elapsed_ms = Some(2_592_000_000);
        value.outcome = Some("completed".into());
        value.coverage = "observed".into();
        value
    }

    fn event(id: usize, run_id: &str, at: i64) -> WorkEvent {
        WorkEvent {
            event_id: format!("bench-event-{id}"),
            run_id: run_id.into(),
            context_ref: format!("context-{}", run_id.trim_start_matches("bench-run-")),
            kind: "progress".into(),
            occurred_at: at,
            observed_at: at,
            phase: Some("running".into()),
            summary: None,
            plan_id: None,
            plan_item_id: None,
            labels: vec![],
            source: "synthetic".into(),
            coverage: "synthetic".into(),
        }
    }

    fn record(sequence: i64, source: String, value: WorkRecord) -> CommittedRecord {
        CommittedRecord {
            sequence,
            source_key: source,
            schema_version: VERSION,
            record: value,
        }
    }

    fn percentile95(values: &mut [u128]) -> u128 {
        values.sort_unstable();
        values[((values.len().saturating_sub(1) * 95) / 100)]
    }

    fn directory_bytes(path: &Path) -> u64 {
        fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum()
    }

    fn emit(name: &str, metrics: serde_json::Value) {
        println!(
            "WORK_LOG_BENCH_JSON {}",
            json!({
                "benchmark": name,
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "buildProfile": option_env!("PROFILE").unwrap_or("unknown"),
                "debugAssertions": cfg!(debug_assertions),
                "metrics": metrics,
            })
        );
    }

    /// Measures real FULL-transaction writer and resource append latency for
    /// synthetic 1/5/20-run workloads. No model or provider is invoked.
    #[test]
    #[ignore = "manual benchmark; emits observed metrics, not a budget assertion"]
    fn synthetic_writer_and_resource_latency() {
        let mut output = Vec::new();
        for logical_runs in [1usize, 5, 20] {
            let dir = BenchDir::new(&format!("writer-{logical_runs}"));
            let db = WorkLogDb::open(&dir.path).expect("open real Work Log database");
            let primary_path = dir.path.join("primary.sqlite3");
            let mut primary = Connection::open(primary_path).expect("open synthetic primary DB");
            primary
                .execute_batch(outbox::DDL)
                .expect("create primary outbox DDL");
            {
                let tx = primary
                    .unchecked_transaction()
                    .expect("configure primary outbox");
                let settings = serde_json::to_string(&CollectionSettings {
                    enabled: true,
                    ..CollectionSettings::default()
                })
                .expect("serialize benchmark settings");
                outbox::set_meta(&tx, "settings", &settings).expect("enable benchmark outbox");
                tx.commit().expect("commit benchmark settings");
            }
            let mut sequence = 1i64;
            let mut tx_ns = Vec::new();
            let mut sample_ns = Vec::new();
            let mut outbox_ns = Vec::new();
            for run_index in 0..logical_runs {
                let run = run(run_index);
                let start = Instant::now();
                let primary_tx = primary
                    .unchecked_transaction()
                    .expect("begin primary run enqueue");
                outbox::append(
                    &primary_tx,
                    &[PendingRecord {
                        source_key: format!("bench-run-{run_index}"),
                        record: WorkRecord::Run(run.clone()),
                    }],
                    false,
                )
                .expect("enqueue synthetic run");
                primary_tx.commit().expect("commit primary run enqueue");
                outbox_ns.push(start.elapsed().as_nanos());
                let start = Instant::now();
                db.apply_batch(&[record(
                    sequence,
                    format!("run-{run_index}"),
                    WorkRecord::Run(run.clone()),
                )])
                .expect("append synthetic run");
                tx_ns.push(start.elapsed().as_nanos());
                sequence += 1;
                for sample in 0..10 {
                    let ev = event(
                        run_index * 10 + sample,
                        &run.run_id,
                        MONTH_START + sample as i64 * 10_000,
                    );
                    let start = Instant::now();
                    let primary_tx = primary
                        .unchecked_transaction()
                        .expect("begin primary event enqueue");
                    outbox::append(
                        &primary_tx,
                        &[PendingRecord {
                            source_key: format!("bench-event-{run_index}-{sample}"),
                            record: WorkRecord::Event(ev.clone()),
                        }],
                        false,
                    )
                    .expect("enqueue synthetic event");
                    primary_tx.commit().expect("commit primary event enqueue");
                    outbox_ns.push(start.elapsed().as_nanos());
                    let start = Instant::now();
                    db.apply_batch(&[record(
                        sequence,
                        format!("event-{run_index}-{sample}"),
                        WorkRecord::Event(ev),
                    )])
                    .expect("append synthetic event");
                    tx_ns.push(start.elapsed().as_nanos());
                    sequence += 1;
                    let resource = ResourceSample {
                        sample_id: format!("resource-{run_index}-{sample}"),
                        run_id: run.run_id.clone(),
                        sampled_at: MONTH_START + sample as i64 * 10_000,
                        interval_start: MONTH_START + sample as i64 * 10_000 - 10_000,
                        covered_ms: 10_000,
                        cpu_delta_ms: Some(5),
                        rss_bytes: Some(1_048_576),
                        rss_byte_ms: Some(10_485_760_000),
                        sampled_peak_rss_bytes: Some(1_048_576),
                        process_count: 1,
                        quality: "complete".into(),
                    };
                    let start = Instant::now();
                    db.append_resources(&[resource])
                        .expect("append synthetic resource sample");
                    sample_ns.push(start.elapsed().as_nanos());
                }
            }
            let tx_p95 = percentile95(&mut tx_ns);
            let sample_p95 = percentile95(&mut sample_ns);
            let outbox_p95 = percentile95(&mut outbox_ns);
            output.push(json!({
                "logicalRuns": logical_runs,
                "writerAppendCount": tx_ns.len(),
                "writerAppendP95Nanos": tx_p95,
                "primaryOutboxEnqueueCount": outbox_ns.len(),
                "primaryOutboxEnqueueP95Nanos": outbox_p95,
                "resourceSampleCount": sample_ns.len(),
                "resourceAppendP95Nanos": sample_p95,
                "dbDirectoryBytes": directory_bytes(&dir.path),
                "latencyIncludes": "synchronous FULL destination transaction; harness scheduling noise",
            }));
        }
        emit("synthetic_writer_and_resource_latency", json!(output));
    }

    /// Ten-minute cadence fixture: measure actual sanitized local process
    /// walks, then transform twenty synthetic verified process trees and
    /// persist each tick as a single resource batch.
    #[test]
    #[ignore = "takes ten minutes; manual collector cadence measurement"]
    fn ten_minute_collector_cadence() {
        let dir = BenchDir::new("ten-minute");
        let db = WorkLogDb::open(&dir.path).expect("open real Work Log database");
        for i in 0..20 {
            db.apply_batch(&[record(
                i + 1,
                format!("run-{i}"),
                WorkRecord::Run(run(i as usize)),
            )])
            .expect("append run");
        }
        let source = LocalProcessSource::new("synthetic-benchmark-host");
        let mut tracker = ResourceTracker::default();
        let roots: Vec<_> = (0..20)
            .map(|i| RunRoot {
                run_id: format!("bench-run-{i}"),
                identity: ProcessIdentity {
                    host_id: "synthetic-benchmark-host".into(),
                    pid: 50_000 + (i as i32 * 2),
                    started_at: 1_000 + i as i64,
                },
                remote: false,
            })
            .collect();
        let usage_before = process_usage_snapshot();
        let started = Instant::now();
        let mut walk_ns = Vec::new();
        let mut transform_ns = Vec::new();
        let mut append_ns = Vec::new();
        let mut process_count_total = 0usize;
        let mut source_failures = 0usize;
        let mut tracker_samples = 0usize;
        for tick in 0..60 {
            let walk_at = Instant::now();
            match source.sample() {
                Ok(observations) => process_count_total += observations.len(),
                Err(_) => source_failures += 1,
            }
            walk_ns.push(walk_at.elapsed().as_nanos());

            let mono_ms = started.elapsed().as_millis() as u64;
            let utc_ms = MONTH_START + mono_ms as i64;
            let mut synthetic = Vec::with_capacity(40);
            for i in 0..20 {
                let root_pid = 50_000 + i as i32 * 2;
                synthetic.push(ProcessObservation {
                    identity: roots[i].identity.clone(),
                    parent_pid: 1,
                    cpu_time_ms: tick as u64 * 2,
                    resident_memory_bytes: 2_097_152,
                });
                synthetic.push(ProcessObservation {
                    identity: ProcessIdentity {
                        host_id: "synthetic-benchmark-host".into(),
                        pid: root_pid + 1,
                        started_at: 2_000 + i as i64,
                    },
                    parent_pid: root_pid,
                    cpu_time_ms: tick as u64 * 3,
                    resident_memory_bytes: 1_048_576,
                });
            }
            let transform_at = Instant::now();
            let samples = tracker.observe(utc_ms, mono_ms, &roots, &synthetic);
            transform_ns.push(transform_at.elapsed().as_nanos());
            tracker_samples += samples.len();
            let append_at = Instant::now();
            db.append_resources(&samples)
                .expect("append tracker sample batch");
            append_ns.push(append_at.elapsed().as_nanos());
            let next_tick = started + Duration::from_secs((tick as u64 + 1) * 10);
            std::thread::sleep(next_tick.saturating_duration_since(Instant::now()));
        }
        let elapsed = started.elapsed();
        let walk_p95 = percentile95(&mut walk_ns);
        let transform_p95 = percentile95(&mut transform_ns);
        let p95 = percentile95(&mut append_ns);
        // Keep output field numeric while recording the observed sample count.
        let usage_after = process_usage_snapshot();
        let delta = |before: Option<u128>, after: Option<u128>| {
            after.zip(before).map(|(a, b)| a.saturating_sub(b))
        };
        let rss_growth = usage_after
            .2
            .zip(usage_before.2)
            .map(|(a, b)| a.saturating_sub(b));
        emit(
            "ten_minute_collector_cadence",
            json!({
                "logicalRuns": 20,
                "cadenceSeconds": 10,
                "samples": 1200,
                "elapsedMs": elapsed.as_millis(),
                "actualLocalProcessWalkCalls": 60,
                "localProcessCountTotalOnly": process_count_total,
                "localProcessSourceFailures": source_failures,
                "localProcessWalkP95Nanos": walk_p95,
                "syntheticVerifiedRootCount": roots.len(),
                "syntheticTrackerSampleCount": tracker_samples,
                "trackerTransformP95Nanos": transform_p95,
                "batchAppendP95Nanos": p95,
                "processCpuUserDeltaMicros": delta(usage_before.0, usage_after.0),
                "processCpuSystemDeltaMicros": delta(usage_before.1, usage_after.1),
                "processMaxRssBytesAtEnd": usage_after.2,
                "processMaxRssGrowthBytes": rss_growth,
                "cpuRssScope": "test process rusage baseline delta/max RSS growth; includes harness and local sampler, not collector-only",
                "dbDirectoryBytes": directory_bytes(&dir.path),
            }),
        );
    }

    /// Builds one million indexed events plus terminal month-long runs,
    /// intervals, usage projections and 20,000 resource samples. Measures
    /// monthly summary and timeline page warm/reopened against real DDL.
    #[test]
    #[ignore = "large one-million-event corpus; manual benchmark"]
    fn million_event_monthly_summary() {
        let dir = BenchDir::new("million-events");
        let path = dir.path.join("work-log.sqlite3");
        let conn = Connection::open(&path).expect("open corpus DB");
        schema::apply_pragmas(&conn).expect("apply storage pragmas");
        schema::apply(&conn).expect("apply real Work Log schema");
        let tx = conn
            .unchecked_transaction()
            .expect("begin corpus transaction");
        for run_index in 0..20usize {
            let value = closed_run(run_index);
            let raw = serde_json::to_string(&value).expect("serialize run fixture");
            tx.execute("INSERT INTO runs(run_id,source_key,task_id,project_id,agent_id,host_id,backend_session_id,context_ref,title,agent_name,host_name,harness,configured_model,remote,started_at,observed_at,ended_at,elapsed_ms,outcome,coverage,raw_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,0,?14,?15,?16,?17,?18,?19,?20)", params![value.run_id, format!("run-source-{run_index}"), value.task_id, value.project_id, value.agent_id, value.host_id, value.backend_session_id, value.context_ref, value.title, value.agent_name, value.host_name, value.harness, value.configured_model, value.started_at, value.observed_at, value.ended_at, value.elapsed_ms.map(|v|i64::try_from(v).unwrap()), value.outcome, value.coverage, raw]).expect("insert corpus run");

            let mut interval = tx.prepare("INSERT INTO work_intervals(interval_id,run_id,kind,started_at,ended_at,duration_ms,coverage,raw_json) VALUES(?1,?2,?3,?4,?5,?6,'observed',?7)").expect("prepare interval insert");
            for day in 0..30i64 {
                for (kind, offset, duration) in [
                    ("runnable", 0i64, 3_600_000i64),
                    ("approvalWait", 3_600_000, 600_000),
                ] {
                    let start = MONTH_START + day * 86_400_000 + offset;
                    let end = start + duration;
                    let item = json!({"intervalId": format!("interval-{run_index}-{day}-{kind}"), "runId": value.run_id.clone(), "kind": kind, "startedAt": start, "endedAt": end, "durationMs": duration, "coverage": "observed"});
                    interval
                        .execute(params![
                            format!("interval-{run_index}-{day}-{kind}"),
                            value.run_id,
                            kind,
                            start,
                            end,
                            duration,
                            item.to_string()
                        ])
                        .expect("insert closed interval");
                }
            }
            let tokens = json!({"input":500_000,"output":200_000,"cacheRead":100_000,"cacheWrite":50_000,"reasoning":40_000,"total":700_000});
            let totals = json!({"runId":value.run_id.clone(),"provider":"synthetic","configuredModel":"synthetic-only","providerTurnId":"turn-fixture","counterEpoch":"initial","tokens":tokens,"reportedCostUnits":1234,"costScale":1000000,"currency":"USD","costSource":"reported","sourcePrecision":"exact","observations":30,"missingObservations":0,"complete":true,"finalSeen":true,"coverage":[]});
            tx.execute("INSERT INTO usage_projection(scope_key,run_id,state_json,totals_json) VALUES(?1,?2,'{}',?3)", params![format!("scope-{run_index}"), value.run_id, totals.to_string()]).expect("insert run usage projection");

            let mut resource = tx.prepare("INSERT INTO resource_samples(sample_id,run_id,sampled_at,interval_start,covered_ms,cpu_delta_ms,rss_bytes,rss_byte_ms,sampled_peak_rss_bytes,process_count,quality,raw_json) VALUES(?1,?2,?3,?4,1000,15,3145728,3145728000,3145728,2,'observed',?5)").expect("prepare resource insert");
            for sample_index in 0..1000i64 {
                let end = MONTH_START + (sample_index + 1) * 2_592_000_000 / 1000;
                let start = end - 1000;
                let item = json!({"sampleId":format!("corpus-resource-{run_index}-{sample_index}"),"runId":value.run_id.clone(),"sampledAt":end,"intervalStart":start,"coveredMs":1000,"cpuDeltaMs":15,"rssBytes":3145728,"rssByteMs":3145728000u64,"sampledPeakRssBytes":3145728,"processCount":2,"quality":"observed"});
                resource
                    .execute(params![
                        format!("corpus-resource-{run_index}-{sample_index}"),
                        value.run_id,
                        end,
                        start,
                        item.to_string()
                    ])
                    .expect("insert corpus resource sample");
            }
        }
        {
            let mut stmt = tx.prepare("INSERT INTO events(event_id,run_id,context_ref,kind,occurred_at,observed_at,phase,labels_json,source,coverage,source_key,sequence,raw_json) VALUES(?1,?2,?3,'progress',?4,?4,'running','[]','synthetic','synthetic',?5,?6,?7)").expect("prepare event insert");
            for i in 0..1_000_000i64 {
                let run_index = (i as usize) % 20;
                let e = event(
                    i as usize,
                    &format!("bench-run-{run_index}"),
                    MONTH_START + (i % 2_592_000) * 1000,
                );
                let raw = serde_json::to_string(&e).expect("serialize event");
                stmt.execute(params![
                    e.event_id,
                    e.run_id,
                    e.context_ref,
                    e.occurred_at,
                    format!("event-source-{i}"),
                    i + 1,
                    raw
                ])
                .expect("insert indexed event");
            }
        }
        tx.commit().expect("commit million-event corpus");
        let plan = conn.prepare("EXPLAIN QUERY PLAN SELECT raw_json FROM runs WHERE COALESCE(ended_at,observed_at)>?1 AND started_at<?2 ORDER BY started_at,run_id LIMIT ?3").expect("prepare plan")
            .query_map(params![MONTH_START, MONTH_START + 2_592_000_000, 10_001], |r| r.get::<_, String>(3)).expect("query plan")
            .collect::<Result<Vec<_>, _>>().expect("read query plan");
        let query = WorkQuery {
            from: Some(MONTH_START),
            through: Some(MONTH_START + 2_592_000_001),
            ..Default::default()
        };
        let warm_started = Instant::now();
        let warm = query::summary(&conn, &query, None).expect("warm indexed monthly summary");
        let warm_ms = warm_started.elapsed().as_millis();
        let page_started = Instant::now();
        let warm_page = query::page(&conn, &query, None).expect("warm monthly timeline page");
        let page_ms = page_started.elapsed().as_millis();
        drop(conn);
        let reopened = Connection::open(&path).expect("reopen corpus");
        let reopen_started = Instant::now();
        let summary = query::summary(&reopened, &query, None).expect("reopened monthly summary");
        let reopen_ms = reopen_started.elapsed().as_millis();
        let reopen_page_started = Instant::now();
        let page = query::page(&reopened, &query, None).expect("reopened monthly timeline page");
        let reopen_page_ms = reopen_page_started.elapsed().as_millis();
        let page_plan = reopened.prepare("EXPLAIN QUERY PLAN SELECT e.sequence,e.source_key,e.event_id,e.raw_json,r.raw_json FROM events e JOIN runs r ON r.run_id=e.run_id WHERE COALESCE(r.ended_at,r.observed_at)>?1 AND r.started_at<?2 AND e.occurred_at>=?1 AND e.occurred_at<?2 ORDER BY e.sequence,e.event_id LIMIT 65").expect("prepare page plan")
            .query_map(params![MONTH_START, MONTH_START + 2_592_000_001], |r| r.get::<_, String>(3)).expect("query page plan")
            .collect::<Result<Vec<_>, _>>().expect("read page plan");
        emit(
            "million_event_monthly_summary",
            json!({
                "events": 1_000_000,
                "runs": summary.runs,
                "warmSummaryMs": warm_ms,
                "reopenedSummaryMs": reopen_ms,
                "warmResultRuns": warm.runs,
                "summaryTargetMs": 250,
                "runnableMs": summary.runnable_ms,
                "knownWaitMs": summary.known_wait_ms,
                "cpuMs": summary.cpu_ms,
                "rssByteMs": summary.rss_byte_ms,
                "projectedTokenInput": summary.tokens.input,
                "reportedCostUnits": summary.reported_cost_units,
                "timelineWarmPageMs": page_ms,
                "timelineReopenedPageMs": reopen_page_ms,
                "timelineWarmEntries": warm_page.entries.len(),
                "timelineReopenedEntries": page.entries.len(),
                "timelinePageQueryPlan": page_plan,
                "queryPlan": plan,
                "dbBytes": fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                "walBytes": fs::metadata(path.with_extension("sqlite3-wal")).map(|m| m.len()).unwrap_or(0),
                "directoryBytes": directory_bytes(&dir.path),
            }),
        );
    }

    /// Deterministic local SQLite stress signals: a held writer lock produces
    /// BUSY at the configured timeout, and max_page_count produces SQLITE_FULL.
    /// This does not make either condition a passing performance budget.
    #[test]
    #[ignore = "manual SQLite stall/full signal; emits observed metrics"]
    fn sqlite_writer_stall_and_full_signals() {
        let dir = BenchDir::new("sqlite-pressure");
        let path = dir.path.join("pressure.sqlite3");
        let writer = Connection::open(&path).expect("open writer");
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=100; CREATE TABLE pressure(k INTEGER PRIMARY KEY, payload BLOB NOT NULL); INSERT INTO pressure VALUES(0,'durable-marker');").expect("create pressure table and marker");
        let reader = Connection::open(&path).expect("open long reader");
        reader
            .execute_batch("PRAGMA busy_timeout=100; BEGIN;")
            .expect("begin long reader");
        let marker_before: String = reader
            .query_row("SELECT payload FROM pressure WHERE k=0", [], |r| r.get(0))
            .expect("establish reader snapshot");
        writer
            .execute_batch("BEGIN IMMEDIATE")
            .expect("hold writer lock");
        let contender = Connection::open(&path).expect("open competing writer");
        contender
            .execute_batch("PRAGMA busy_timeout=100;")
            .expect("configure competing writer");
        let started = Instant::now();
        let busy_error = contender
            .execute("INSERT INTO pressure VALUES(1,zeroblob(64))", [])
            .expect_err("competing writer must block behind immediate writer transaction");
        let blocked_ms = started.elapsed().as_millis();
        writer
            .execute_batch("ROLLBACK")
            .expect("release writer lock");
        for i in 0..16i64 {
            writer
                .execute("INSERT INTO pressure VALUES(?1,randomblob(128))", [i + 2])
                .expect("write while old reader pins WAL");
        }
        let (pinned_busy, pinned_frames, pinned_checkpointed): (i64, i64, i64) = writer
            .query_row("PRAGMA wal_checkpoint(FULL)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .expect("observe checkpoint while reader is active");
        assert!(
            pinned_busy != 0 || pinned_checkpointed < pinned_frames,
            "long reader must pin at least part of the WAL checkpoint"
        );
        reader
            .execute_batch("ROLLBACK")
            .expect("release long reader");
        let (released_busy, released_frames, released_checkpointed): (i64, i64, i64) = writer
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .expect("checkpoint after reader release");
        assert_eq!(
            released_busy, 0,
            "checkpoint should be unblocked after releasing reader"
        );
        assert_eq!(
            released_frames, released_checkpointed,
            "all WAL frames should checkpoint after reader release"
        );

        writer
            .execute_batch("PRAGMA max_page_count=4; PRAGMA cache_size=1;")
            .expect("cap pages");
        let mut full_error = None;
        for i in 0..1000i64 {
            match writer.execute(
                "INSERT INTO pressure VALUES(?1,randomblob(4096))",
                [i + 1000],
            ) {
                Ok(_) => {}
                Err(error) => {
                    full_error = Some(error);
                    break;
                }
            }
        }
        let is_busy = matches!(&busy_error, SqliteError::SqliteFailure(e, _) if matches!(e.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked));
        let full_debug = full_error.as_ref().map(|e| format!("{e:?}"));
        let is_full = matches!(&full_error, Some(SqliteError::SqliteFailure(e, _)) if e.code == ErrorCode::DiskFull);
        assert!(is_busy, "expected SQLITE_BUSY/LOCKED, got {busy_error:?}");
        assert!(is_full, "expected SQLITE_FULL, got {full_debug:?}");
        let marker_after: String = writer
            .query_row("SELECT payload FROM pressure WHERE k=0", [], |r| r.get(0))
            .expect("read durable marker after pressure");
        assert_eq!(marker_before, "durable-marker");
        assert_eq!(
            marker_after, marker_before,
            "pressure errors must preserve the durable marker"
        );
        emit(
            "sqlite_writer_stall_and_full_signals",
            json!({
                "busyElapsedMs": blocked_ms,
                "busyError": format!("{busy_error:?}"),
                "fullError": full_debug,
                "busyCodeVerified": is_busy,
                "fullCodeVerified": is_full,
                "durableMarkerPreserved": marker_after == marker_before,
                "pinnedReaderCheckpoint": {"busy":pinned_busy,"logFrames":pinned_frames,"checkpointedFrames":pinned_checkpointed},
                "releasedReaderCheckpoint": {"busy":released_busy,"logFrames":released_frames,"checkpointedFrames":released_checkpointed},
                "dbBytes": fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                "walBytes": fs::metadata(path.with_extension("sqlite3-wal")).map(|m| m.len()).unwrap_or(0),
            }),
        );
    }

    // getrusage reports cumulative process totals and maximum resident size.
    // On macOS ru_maxrss is bytes; on Linux it is KiB, so normalize by target.
    #[cfg(unix)]
    fn process_usage_snapshot() -> (Option<u128>, Option<u128>, Option<u64>) {
        unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
                return (None, None, None);
            }
            let timeval_us = |t: libc::timeval| t.tv_sec as u128 * 1_000_000 + t.tv_usec as u128;
            let rss = usage.ru_maxrss.max(0) as u64;
            let rss = if cfg!(target_os = "macos") {
                rss
            } else {
                rss.saturating_mul(1024)
            };
            (
                Some(timeval_us(usage.ru_utime)),
                Some(timeval_us(usage.ru_stime)),
                Some(rss),
            )
        }
    }
    #[cfg(not(unix))]
    fn process_usage_snapshot() -> (Option<u128>, Option<u128>, Option<u64>) {
        (None, None, None)
    }
}
