//! Manual, synthetic end-to-end latency evidence for the native state path.
//! Run explicitly with `cargo test --lib service_latency_benchmark -- --ignored
//! --nocapture`; it never opens a harness, model, or user data directory.

use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn percentile(values: &[Duration], percentile: f64) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[((sorted.len() - 1) as f64 * percentile).round() as usize]
}

fn tree_bytes(path: &Path) -> u64 {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                tree_bytes(&path)
            } else {
                entry.metadata().map(|metadata| metadata.len()).unwrap_or(0)
            }
        })
        .sum()
}

fn benchmark_event(id: String, detail: Arc<str>) -> Arc<RunEvent> {
    Arc::new(RunEvent {
        id,
        task_id: "benchmark-task".into(),
        kind: "tool".into(),
        title: "Synthetic streaming activity".into(),
        detail,
        created_at: now(),
    })
}

#[test]
#[ignore = "manual native Service mutation and UI projection benchmark"]
fn large_history_mutation_and_ui_snapshot_latency() {
    const DEFAULT_EVENTS: usize = 4_096;
    const DEFAULT_MESSAGES: usize = 1_000;
    const EVENT_BYTES: usize = 4_096;
    const MESSAGE_BYTES: usize = 2_048;
    const SAMPLES: usize = 20;

    let events = std::env::var("MONITTER_BENCH_EVENTS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_EVENTS);
    let messages = if events >= 53_000 {
        4_000
    } else {
        DEFAULT_MESSAGES
    };
    let root = TempRoot(std::env::temp_dir().join(format!("monitter-service-latency-{}", id())));
    let service = Service::open(None, root.0.clone()).unwrap();
    let message_text = "m".repeat(MESSAGE_BYTES);

    // Seed through the production transactional writer path before measuring
    // small changes against the same complete historical state.
    service
        .mutate_data(None, |data| {
            data.snapshot.events = (0..events)
                // Separate allocations match events loaded from JSON; only
                // later snapshot clones share those immutable allocations.
                .map(|index| {
                    benchmark_event(
                        format!("fixture-event-{index}"),
                        "e".repeat(EVENT_BYTES).into(),
                    )
                })
                .collect();
            data.snapshot.messages = (0..messages)
                .map(|index| Message {
                    stream_status: Some("streaming".into()),
                    phase: None,
                    response_metadata: None,
                    id: format!("fixture-message-{index}"),
                    task_id: "benchmark-task".into(),
                    role: "assistant".into(),
                    text: message_text.clone(),
                    created_at: index as i64,
                    sender_agent_id: None,
                    collaboration_id: None,
                    attachments: vec![],
                })
                .collect();
            Ok(())
        })
        .unwrap();
    {
        let _writer = service.state_writes.lock().unwrap();
        let data = service.committed_data().unwrap();
        service
            .store
            .save(&data.snapshot, &data.task_hosts, &data.attachments)
            .unwrap();
    }

    // Warm allocation, filesystem, and compact-projection paths without
    // including this first use in the evidence.
    service
        .mutate_data(None, |data| {
            data.snapshot.messages.last_mut().unwrap().text.push('w');
            data.snapshot
                .events
                .push(benchmark_event("warm-event".into(), Arc::from("warm")));
            Ok(())
        })
        .unwrap();
    let warm_revision = service.ui_snapshot(None).unwrap().revision;
    let _ = service.ui_snapshot(Some(&warm_revision)).unwrap();
    let _ = service.ui_snapshot(None).unwrap();

    let mut mutation_times = Vec::with_capacity(SAMPLES);
    let mut cached_read_times = Vec::with_capacity(SAMPLES);
    let mut projection_times = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let started = Instant::now();
        service
            .mutate_data(None, |data| {
                data.snapshot.messages.last_mut().unwrap().text.push('x');
                data.snapshot.events.push(benchmark_event(
                    format!("stream-event-{sample}"),
                    Arc::from("stream"),
                ));
                Ok(())
            })
            .unwrap();
        mutation_times.push(started.elapsed());

        let revision = service.ui_snapshot(None).unwrap().revision;
        let started = Instant::now();
        let cached = service.ui_snapshot(Some(&revision)).unwrap();
        cached_read_times.push(started.elapsed());
        assert!(cached.snapshot.is_none());

        let started = Instant::now();
        let projected = service.ui_snapshot(None).unwrap();
        projection_times.push(started.elapsed());
        assert!(projected.snapshot.is_some());
    }

    let database_bytes = fs::metadata(root.0.join("state.sqlite3"))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let total_bytes = tree_bytes(&root.0);
    eprintln!(
        "native-latency events={events} messages={messages} samples={SAMPLES} \\
mutation_p50={:?} mutation_p95={:?} cached_ui_p50={:?} cached_ui_p95={:?} \\
projection_p50={:?} projection_p95={:?} database_bytes={database_bytes} total_bytes={total_bytes}",
        percentile(&mutation_times, 0.50),
        percentile(&mutation_times, 0.95),
        percentile(&cached_read_times, 0.50),
        percentile(&cached_read_times, 0.95),
        percentile(&projection_times, 0.50),
        percentile(&projection_times, 0.95),
    );

    drop(service);
}
