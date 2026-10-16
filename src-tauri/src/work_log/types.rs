use crate::model::{RunUsageSample, UsageTokens};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const OUTBOX_MAX_ROWS: usize = 10_000;
pub const OUTBOX_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const RECORD_MAX_BYTES: usize = 32 * 1024;
pub const RESOURCE_MAX_ROWS: usize = 4096;
pub const RESOURCE_MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionSettings {
    pub enabled: bool,
    pub raw_retention_days: u32,
    pub minute_retention_days: u32,
    pub soft_limit_bytes: u64,
    pub hard_limit_bytes: u64,
}
impl Default for CollectionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            raw_retention_days: 30,
            minute_retention_days: 365,
            soft_limit_bytes: 1024 * 1024 * 1024,
            hard_limit_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}
impl CollectionSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.raw_retention_days > 365
            || self.minute_retention_days < self.raw_retention_days
            || self.minute_retention_days > 3650
            || self.soft_limit_bytes < 1024 * 1024
            || self.hard_limit_bytes < self.soft_limit_bytes
            || self.hard_limit_bytes > 64 * 1024 * 1024 * 1024
        {
            return Err("Invalid work log collection limits.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContextAvailability {
    Available,
    Deleted,
    NotRecorded,
    Unsupported,
    TemporarilyUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextRef {
    pub context_ref: String,
    pub installation_id: String,
    pub profile_id: String,
    pub project_id: Option<String>,
    pub task_id: String,
    pub run_id: String,
    pub provider_turn_id: Option<String>,
    /// Durable context-cleared marker or first message; never a title lookup.
    pub segment_start_id: Option<String>,
    pub source_message_id: Option<String>,
    pub through_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_message_text_bytes: Option<u32>,
    pub title: String,
    pub agent_name: String,
    pub harness: String,
    /// Internal provider ID only; never a path and never returned to agent tools.
    pub native_session_id: Option<String>,
    #[serde(default)]
    pub lineage_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_root_task_id: Option<String>,
    #[serde(default)]
    pub source_task_id: Option<String>,
    #[serde(default)]
    pub source_through_message_id: Option<String>,
    pub availability: ContextAvailability,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkRun {
    pub run_id: String,
    pub task_id: String,
    pub project_id: Option<String>,
    pub agent_id: String,
    pub host_id: String,
    pub backend_session_id: String,
    pub parent_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub context_ref: String,
    pub title: String,
    pub project_name: Option<String>,
    pub agent_name: String,
    pub host_name: String,
    pub harness: String,
    pub configured_model: Option<String>,
    pub observed_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_setting: Option<String>,
    #[serde(
        default = "unknown_trigger",
        skip_serializing_if = "is_unknown_trigger"
    )]
    pub trigger: String,
    pub remote: bool,
    pub started_at: i64,
    pub observed_at: i64,
    pub ended_at: Option<i64>,
    pub elapsed_ms: Option<u64>,
    pub outcome: Option<String>,
    pub coverage: String,
}

fn is_unknown_trigger(value: &str) -> bool {
    value == "unknown"
}
fn unknown_trigger() -> String {
    "unknown".into()
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkInterval {
    pub interval_id: String,
    pub run_id: String,
    pub kind: String,
    pub started_at: i64,
    pub ended_at: i64,
    pub duration_ms: u64,
    pub coverage: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkEvent {
    pub event_id: String,
    pub run_id: String,
    pub context_ref: String,
    pub kind: String,
    pub occurred_at: i64,
    pub observed_at: i64,
    pub phase: Option<String>,
    pub summary: Option<String>,
    pub plan_id: Option<String>,
    pub plan_item_id: Option<String>,
    pub labels: Vec<String>,
    pub source: String,
    pub coverage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "record", rename_all = "camelCase")]
pub enum WorkRecord {
    Run(WorkRun),
    Context(ContextRef),
    Event(WorkEvent),
    Usage(RunUsageSample),
    Interval(WorkInterval),
    Gap(CollectorGap),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommittedRecord {
    /// Primary AUTOINCREMENT sequence; distinct from destination watermark.
    pub sequence: i64,
    pub source_key: String,
    pub schema_version: u32,
    pub record: WorkRecord,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingRecord {
    pub source_key: String,
    pub record: WorkRecord,
}

/// Implemented by the primary Store, not by a frontend or remote caller.
pub trait OutboxSource {
    fn pending(&self, limit: usize) -> Result<Vec<CommittedRecord>, String>;
    /// Called only after destination FULL commit. Replay is harmless.
    fn acknowledge(&self, through_sequence: i64) -> Result<(), String>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectorGap {
    pub gap_id: String,
    pub run_id: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub reason: String,
    pub dropped: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectorHealth {
    pub state: String,
    pub reason: Option<String>,
    pub last_write_at: Option<i64>,
    pub pending_rows: u64,
    pub pending_bytes: u64,
    pub dropped_resources: u64,
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub watermark: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct ProcessIdentity {
    pub host_id: String,
    pub pid: i32,
    pub started_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessObservation {
    pub identity: ProcessIdentity,
    pub parent_pid: i32,
    pub cpu_time_ms: u64,
    pub resident_memory_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunRoot {
    pub run_id: String,
    pub identity: ProcessIdentity,
    pub remote: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSample {
    pub sample_id: String,
    pub run_id: String,
    pub sampled_at: i64,
    pub interval_start: i64,
    pub covered_ms: u64,
    pub cpu_delta_ms: Option<u64>,
    pub rss_bytes: Option<u64>,
    pub rss_byte_ms: Option<u64>,
    pub sampled_peak_rss_bytes: Option<u64>,
    pub process_count: u32,
    pub quality: String,
}
pub trait Clock: Send + Sync {
    fn utc_ms(&self) -> i64;
    fn monotonic_ms(&self) -> u64;
}
pub trait ProcessSource: Send + Sync {
    fn sample(&self) -> Result<Vec<ProcessObservation>, String>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageObservation {
    pub sequence: i64,
    pub sample: RunUsageSample,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageProjection {
    pub run_id: String,
    pub provider: String,
    pub configured_model: Option<String>,
    pub provider_turn_id: Option<String>,
    pub counter_epoch: String,
    pub tokens: UsageTokens,
    pub reported_cost_units: Option<i64>,
    pub cost_scale: u32,
    pub currency: Option<String>,
    pub cost_source: Option<String>,
    pub source_precision: Option<String>,
    pub observations: u64,
    pub missing_observations: u64,
    pub complete: bool,
    pub final_seen: bool,
    pub coverage: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkQuery {
    pub timezone: Option<String>,
    pub from: Option<i64>,
    pub through: Option<i64>,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub host_id: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub phase: Option<String>,
    pub outcome: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkEntry {
    pub sequence: i64,
    pub source_key: String,
    pub run: WorkRun,
    pub event: WorkEvent,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkPage {
    pub entries: Vec<WorkEntry>,
    pub next_cursor: Option<String>,
    pub coverage: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkDay {
    pub date: String,
    pub timezone: String,
    pub agent_elapsed_ms: u64,
    pub project_covered_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkSummary {
    pub days: Vec<WorkDay>,
    pub runs: u64,
    pub agent_elapsed_ms: u64,
    pub project_covered_ms: u64,
    pub known_wait_ms: Option<u64>,
    pub runnable_ms: Option<u64>,
    pub cpu_ms: Option<u64>,
    pub rss_byte_ms: Option<u64>,
    pub covered_resource_ms: u64,
    pub sampled_peak_rss_bytes: Option<u64>,
    pub tokens: UsageTokens,
    pub missing_usage_runs: u64,
    pub reported_cost_units: Option<i64>,
    pub cost_scale: u32,
    pub currency: Option<String>,
    pub coverage: Vec<String>,
    pub resolution: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportInput {
    pub request_id: String,
    pub phase: String,
    pub summary: String,
    #[serde(default)]
    pub plan_item_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReportReceipt {
    pub source_key: String,
    pub sequence: i64,
    pub accepted_at: i64,
}

#[cfg(test)]
pub mod fixtures {
    use super::*;
    pub struct FakeClock {
        pub utc: i64,
        pub monotonic: u64,
    }
    impl Clock for FakeClock {
        fn utc_ms(&self) -> i64 {
            self.utc
        }
        fn monotonic_ms(&self) -> u64 {
            self.monotonic
        }
    }
    pub struct FakeProcesses(pub Vec<ProcessObservation>);
    impl ProcessSource for FakeProcesses {
        fn sample(&self) -> Result<Vec<ProcessObservation>, String> {
            Ok(self.0.clone())
        }
    }
    pub fn run(id: &str) -> WorkRun {
        WorkRun {
            run_id: id.into(),
            task_id: "task".into(),
            project_id: Some("project".into()),
            agent_id: "agent".into(),
            host_id: "host".into(),
            backend_session_id: "session".into(),
            parent_run_id: None,
            parent_task_id: None,
            context_ref: format!("context-{id}"),
            title: "Fixture".into(),
            project_name: Some("Project".into()),
            agent_name: "Agent".into(),
            host_name: "Host".into(),
            harness: "codex".into(),
            configured_model: None,
            observed_model: None,
            cwd: None,
            repository_root: None,
            commit_sha: None,
            adapter_version: None,
            reasoning_setting: None,
            trigger: "unknown".into(),
            remote: false,
            started_at: 1000,
            observed_at: 1000,
            ended_at: None,
            elapsed_ms: None,
            outcome: None,
            coverage: "observed".into(),
        }
    }
}
