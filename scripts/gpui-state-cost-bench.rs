//! Measures the actual `collaboration::records` production implementation at
//! baseline commit ed040e6 and in the current worktree. `build.rs` extracts
//! `field`, `rows`/`rows_ref`, `records`, and `human_name` from both sources on
//! every build, so rerun after edits to collaboration.rs. It also compares the
//! two implementations on empty, nested/cyclic, and fallback fixtures.
//!
//! Synthetic workload: 256, 2,048, and 10,000 unrelated message rows and event
//! rows, each with a 2 KiB text/detail field, plus four real child sessions and
//! collaborations under parent task "parent". Both versions must return equal
//! records before timings are printed. Reports median and nearest-rank p95 over
//! 31 samples after one warmup. `cached_items_len` measures the new pane badge
//! operation (`self.items.len()`) after the records were already computed.
//! `serde_to_value` and `json_clone` measure frontend full projection/copy calls
//! against the same JSON snapshot; using JSON avoids building all Monitter types.
//!
//! From repository root, using only cached crates:
//!   cargo run --offline --manifest-path scripts/gpui-state-cost-bench/Cargo.toml
//!   cargo run --offline --release --manifest-path scripts/gpui-state-cost-bench/Cargo.toml
//! The first is debug; the second is optimized. This does not build the app or
//! read live profile/database data. Cargo may compile cached serde dependencies.

use serde_json::{json, Value};
use std::hint::black_box;
use std::time::{Duration, Instant};

mod baseline {
    include!(concat!(env!("OUT_DIR"), "/baseline_records.rs"));
}
mod current {
    include!(concat!(env!("OUT_DIR"), "/current_records.rs"));
}

const BODY_BYTES: usize = 2 * 1024;
const SAMPLES: usize = 31;

fn fixture_empty() -> Value {
    json!({
        "subagentSessions": [], "collaborations": [], "tasks": [],
        "agents": [], "projects": [], "events": [], "messages": []
    })
}

fn fixture_nested_cycle() -> Value {
    json!({
        "subagentSessions": [
            {"id":"s1","source":"delegation","parentTaskId":"parent","collaborationId":"c1","status":"running","createdAt":100,"updatedAt":110},
            {"id":"s2","source":"delegation","parentTaskId":"child-1","collaborationId":"c2","status":"completed","createdAt":120,"updatedAt":130}
        ],
        "collaborations": [
            {"id":"c1","kind":"delegation","fromTaskId":"parent","toTaskId":"child-1","toAgentId":"a1","text":"first","status":"running","createdAt":100,"updatedAt":110},
            {"id":"c2","kind":"delegation","fromTaskId":"child-1","toTaskId":"child-2","toAgentId":"a2","text":"nested","status":"completed","createdAt":120,"updatedAt":130},
            {"id":"c3","kind":"delegation","fromTaskId":"child-2","toTaskId":"parent","toAgentId":"a3","text":"cycle","status":"completed","createdAt":140,"updatedAt":150},
            {"id":"orphan","kind":"delegation","fromTaskId":"parent","toTaskId":"child-3","toAgentId":"a3","text":"legacy fallback","status":"queued","createdAt":160,"updatedAt":170}
        ],
        "tasks": [
            {"id":"child-1","title":"First child","agentId":"a1","model":"default","createdAt":100,"updatedAt":110},
            {"id":"child-2","title":"Nested child","agentId":"a2","model":"default","createdAt":120,"updatedAt":130}
        ],
        "agents": [{"id":"a1","name":"Research"},{"id":"a2","name":"Writer"}],
        "projects": [], "events": [], "messages": []
    })
}

fn fixture_fallbacks() -> Value {
    json!({
        "subagentSessions": [{
            "id":"native","source":"codex","parentTaskId":"parent",
            "agentThreadId":"thread","agentPath":"/agents/agent-research_helper",
            "status":"completed","prompt":"Fallback title\nignored line",
            "createdAt":100,"updatedAt":200
        }],
        "collaborations": [], "tasks": [], "agents": [], "projects": [],
        "events": [], "messages": []
    })
}

fn assert_equivalent_fixtures() {
    for (name, snapshot) in [
        ("empty", fixture_empty()),
        ("nested_cycle", fixture_nested_cycle()),
        ("fallbacks", fixture_fallbacks()),
    ] {
        let before = baseline::records(&snapshot, "parent");
        let after = current::records(&snapshot, "parent");
        assert_eq!(before, after, "baseline/current mismatch for {name}");
    }
}

fn large_snapshot(row_count: usize) -> Value {
    let text = "m".repeat(BODY_BYTES);
    let detail = "e".repeat(BODY_BYTES);
    let messages: Vec<_> = (0..row_count)
        .map(|index| json!({
            "id":format!("message-{index:08}"), "taskId":"unrelated-task",
            "role":"assistant", "text":text, "createdAt":index as i64,
            "responseMetadata":null
        }))
        .collect();
    let mut events: Vec<_> = (0..row_count)
        .map(|index| json!({
            "id":format!("event-{index:08}"), "taskId":"unrelated-task",
            "kind":"tool", "detail":detail, "createdAt":index as i64,
            "title":"Unrelated event"
        }))
        .collect();
    // Relevant parent events exercise the production JSON detail parser too.
    for index in 0..4 {
        events.push(json!({
            "id":format!("parent-event-{index}"), "taskId":"parent",
            "kind":"subagent", "detail":format!("{{\"activity\":{{\"agentThreadId\":\"other-{index}\"}}}}{detail}"),
            "createdAt":(row_count + index) as i64, "title":"Parent activity"
        }));
    }
    let sessions: Vec<_> = (0..4)
        .map(|index| json!({
            "id":format!("session-{index}"), "source":"delegation",
            "parentTaskId":"parent", "collaborationId":format!("collab-{index}"),
            "agentThreadId":format!("thread-{index}"), "status":"completed",
            "createdAt":index as i64, "updatedAt":index as i64,
            "prompt":format!("Child {index}")
        }))
        .collect();
    let collaborations: Vec<_> = (0..4)
        .map(|index| json!({
            "id":format!("collab-{index}"), "kind":"delegation",
            "fromTaskId":"parent", "toTaskId":format!("child-{index}"),
            "toAgentId":format!("agent-{index}"), "text":format!("Child prompt {index}"),
            "status":"completed", "createdAt":index as i64, "updatedAt":index as i64
        }))
        .collect();
    let tasks: Vec<_> = (0..4)
        .map(|index| json!({
            "id":format!("child-{index}"), "title":format!("Child task {index}"),
            "agentId":format!("agent-{index}"), "model":"default",
            "createdAt":index as i64, "updatedAt":index as i64
        }))
        .collect();
    let agents: Vec<_> = (0..4)
        .map(|index| json!({"id":format!("agent-{index}"), "name":format!("Agent {index}"), "model":"test-model"}))
        .collect();

    json!({
        "subagentSessions":sessions, "collaborations":collaborations,
        "tasks":tasks, "agents":agents, "projects":[],
        "events":events, "messages":messages
    })
}

fn summarize(mut values: Vec<Duration>) -> (Duration, Duration) {
    values.sort_unstable();
    let median = values[values.len() / 2];
    let p95_rank = (values.len() * 95).div_ceil(100).max(1);
    (median, values[p95_rank - 1])
}

fn time<T>(mut operation: impl FnMut() -> T) -> (Duration, Duration) {
    black_box(operation());
    let mut values = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        black_box(operation());
        values.push(start.elapsed());
    }
    summarize(values)
}

fn ms(value: Duration) -> f64 {
    value.as_secs_f64() * 1_000.0
}

fn report(name: &str, messages: usize, events: usize, timing: (Duration, Duration)) {
    println!(
        "{name} {messages} {events} {BODY_BYTES} {:.3} {:.3}",
        ms(timing.0),
        ms(timing.1)
    );
}

fn main() {
    assert_equivalent_fixtures();
    println!("stage messages events bytes_per_row median_ms p95_ms");
    for count in [256, 2_048, 10_000] {
        let snapshot = large_snapshot(count);
        let before = baseline::records(&snapshot, "parent");
        let after = current::records(&snapshot, "parent");
        assert_eq!(before, after, "baseline/current mismatch at row count {count}");
        assert_eq!(before.len(), after.len());

        let baseline_timing = time(|| baseline::records(black_box(&snapshot), "parent"));
        let current_timing = time(|| current::records(black_box(&snapshot), "parent"));
        let cached_items_len = time(|| black_box(after.len()));
        let serde_timing = time(|| serde_json::to_value(black_box(&snapshot)).unwrap());
        let projected: Value = serde_json::to_value(&snapshot).unwrap();
        let clone_timing = time(|| black_box(projected.clone()));
        report("baseline_records", count, count + 4, baseline_timing);
        report("current_records", count, count + 4, current_timing);
        report("cached_items_len", count, count + 4, cached_items_len);
        report("serde_to_value", count, count + 4, serde_timing);
        report("json_clone", count, count + 4, clone_timing);
    }
}
