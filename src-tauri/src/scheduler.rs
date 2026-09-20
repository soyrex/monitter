//! In-process scheduled agent runs.
//!
//! The scheduler owns a list of `Schedule` rows and a single
//! background thread that computes the next fire across every
//! enabled schedule, sleeps until the earliest, then dispatches.
//! It lives inside the same process as the desktop app — if
//! Monitter is not running, no schedule fires. That property is
//! intentional and documented in `CONTRACT.md`.
//!
//! The contract also says: fires are a Rust-side decision. The MCP
//! and Tauri command surfaces are configuration only. A schedule
//! row's `mode` and `frequency` decide what happens; no chat-driven
//! code path can bypass the scheduler loop to "force fire now"
//! without going through `run_schedule_now`, which still records a
//! run and respects the schedule's overlap policy.

use crate::model::{
    Agent, OverlapPolicy, Schedule, ScheduleMode, ScheduleRun, Snapshot,
};
use chrono_tz::Tz;
use croner::Cron;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// How long the scheduler thread sleeps between wake-ups. The loop
/// recomputes next-fire every tick so schedule mutations are picked
/// up promptly without an event channel.
pub(crate) const SCHEDULER_TICK: std::time::Duration = std::time::Duration::from_secs(15);

/// Friendly preset constructors. The wire surface (and the chat-MCP
/// surface) accepts one of these names; the scheduler translates to
/// a 5-field cron expression at write time and never persists the
/// preset. Cron is the only source of truth on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulePreset {
    Every5Minutes,
    Every15Minutes,
    Every30Minutes,
    Hourly,
    Daily9am,
    WeekdayMornings,
    WeeklyMonday,
    MonthlyFirst,
}

impl SchedulePreset {
    pub fn cron(self) -> &'static str {
        match self {
            Self::Every5Minutes => "*/5 * * * *",
            Self::Every15Minutes => "*/15 * * * *",
            Self::Every30Minutes => "*/30 * * * *",
            Self::Hourly => "0 * * * *",
            Self::Daily9am => "0 9 * * *",
            Self::WeekdayMornings => "0 9 * * 1-5",
            Self::WeeklyMonday => "0 9 * * 1",
            Self::MonthlyFirst => "0 9 1 * *",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "every_5_minutes" => Some(Self::Every5Minutes),
            "every_15_minutes" => Some(Self::Every15Minutes),
            "every_30_minutes" => Some(Self::Every30Minutes),
            "hourly" => Some(Self::Hourly),
            "daily_9am" => Some(Self::Daily9am),
            "weekday_mornings" => Some(Self::WeekdayMornings),
            "weekly_monday" => Some(Self::WeeklyMonday),
            "monthly_first" => Some(Self::MonthlyFirst),
            _ => None,
        }
    }
}

/// Wire-level input shape for the Tauri commands. `preset` and
/// `frequency` are mutually exclusive; if both are present,
/// `preset` wins. Either is translated to a stored cron expression
/// before persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleInput {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    pub prompt: String,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub frequency: Option<String>,
    #[serde(default = "default_tz")]
    pub tz: String,
    #[serde(default = "default_mode")]
    pub mode: ScheduleMode,
    #[serde(default)]
    pub overlap_policy: Option<OverlapPolicy>,
    #[serde(default)]
    pub max_consecutive_failures: Option<u32>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_tz() -> String {
    "host".into()
}

fn default_mode() -> ScheduleMode {
    ScheduleMode::PersistentThread
}

fn default_enabled() -> bool {
    true
}

impl ScheduleInput {
    /// Resolves the canonical cron expression from either `preset` or
    /// `frequency`. Returns a clear error when neither is supplied or
    /// both are invalid.
    pub fn resolve_frequency(&self) -> Result<String, String> {
        if let Some(preset) = self.preset.as_deref() {
            return SchedulePreset::parse(preset)
                .map(|p| p.cron().to_string())
                .ok_or_else(|| {
                    format!(
                        "Unknown schedule preset '{preset}'. Use a known preset or supply a raw 'frequency' cron string."
                    )
                });
        }
        if let Some(freq) = self.frequency.as_deref() {
            validate_cron(freq)?;
            return Ok(freq.to_string());
        }
        Err("Schedule must specify either 'preset' or 'frequency'.".into())
    }

    /// Validates the input and projects it to a storable `Schedule`
    /// row. `created_at` and `updated_at` use `model::now()`.
    pub fn into_schedule(self) -> Result<Schedule, String> {
        let frequency = self.resolve_frequency()?;
        let overlap_policy = self
            .overlap_policy
            .unwrap_or(default_overlap_policy_for(self.mode));
        let mut schedule = Schedule {
            id: self.id,
            title: self.title,
            agent_id: self.agent_id,
            prompt: self.prompt,
            frequency,
            tz: self.tz,
            mode: self.mode,
            overlap_policy,
            max_consecutive_failures: self.max_consecutive_failures,
            enabled: self.enabled,
            last_fire_at_ms: None,
            consecutive_failures: 0,
            created_at: crate::model::now(),
            updated_at: crate::model::now(),
        };
        schedule.normalise()?;
        Ok(schedule)
    }
}

fn default_overlap_policy_for(mode: ScheduleMode) -> OverlapPolicy {
    match mode {
        ScheduleMode::PersistentThread => OverlapPolicy::Queue,
        _ => OverlapPolicy::Skip,
    }
}

impl Schedule {
    pub fn normalise(&mut self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("Schedule title is required.".into());
        }
        if self.agent_id.trim().is_empty() {
            return Err("Schedule must reference an agent.".into());
        }
        if self.prompt.trim().is_empty() {
            return Err("Schedule prompt is required.".into());
        }
        validate_cron(&self.frequency)?;
        if self.tz != "host" && Tz::from_str(&self.tz).is_err() {
            return Err(format!(
                "Schedule timezone '{}' is not 'host' or a known IANA name.",
                self.tz
            ));
        }
        if self.mode != ScheduleMode::PersistentThread
            && self.overlap_policy == OverlapPolicy::Queue
        {
            // Queue only makes sense for persistent-thread.
            self.overlap_policy = OverlapPolicy::Skip;
        }
        Ok(())
    }

    /// Returns the next firing instant strictly after `from_ms` in
    /// the schedule's resolved timezone. `None` if the cron
    /// expression cannot be evaluated. For `tz == "host"`, the
    /// schedule fires against the OS local zone (`chrono::Local`).
    pub fn next_fire_after(&self, from_ms: i64) -> Option<i64> {
        let cron = Cron::from_str(&self.frequency).ok()?;
        let from_utc = millis_to_datetime(from_ms);
        if self.tz == "host" {
            let from_local = from_utc.with_timezone(&chrono::Local);
            cron.find_next_occurrence(&from_local, false)
                .ok()
                .map(|dt| dt.to_utc().timestamp_millis())
        } else {
            let tz: Tz = self.tz.parse().ok()?;
            let from_zoned = from_utc.with_timezone(&tz);
            cron.find_next_occurrence(&from_zoned, false)
                .ok()
                .map(|dt| dt.to_utc().timestamp_millis())
        }
    }
}

/// Validates a 5-field cron expression. Returns the canonical error
/// from the `croner` crate if the expression cannot be parsed.
pub fn validate_cron(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("Cron expression is required.".into());
    }
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!(
            "Cron expression must have exactly 5 fields (m h dom mon dow); got {}.",
            parts.len()
        ));
    }
    Cron::from_str(value)
        .map(|_| ())
        .map_err(|e| format!("Invalid cron expression '{}': {e}", value))
}

/// Resolves the agent referenced by the schedule. Errors visibly
/// when the agent is missing or is the internal Monitter Admin
/// agent.
pub fn resolve_schedule_agent<'a>(
    snapshot: &'a Snapshot,
    agent_id: &str,
) -> Result<&'a Agent, String> {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| format!("Schedule agent '{agent_id}' was not found."))?;
    if agent.internal {
        return Err(
            "Schedules cannot target the internal Monitter Admin agent.".into(),
        );
    }
    Ok(agent)
}

/// Returns the n most recent runs for a schedule, newest first.
pub fn recent_runs_for_schedule<'a>(
    runs: &'a [ScheduleRun],
    schedule_id: &str,
    limit: usize,
) -> Vec<&'a ScheduleRun> {
    let mut filtered: Vec<&ScheduleRun> = runs
        .iter()
        .filter(|run| run.schedule_id == schedule_id)
        .collect();
    filtered.sort_by_key(|run| std::cmp::Reverse(run.fired_at_ms));
    filtered.truncate(limit);
    filtered
}

/// Decision returned by `decide_overlap` so the caller can either
/// fire, skip, or queue without re-inspecting the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlapDecision {
    /// Previous fire is still running; the schedule's overlap policy
    /// is `Skip`. Record a `skipped_overlap: true` run and move on.
    Skip,
    /// Previous fire is still running and the schedule's overlap
    /// policy is `Queue`. Reuse the existing task and append the
    /// prompt to it. Caller still records a run with the existing
    /// `task_id`.
    Queue,
    /// Either no previous fire is running, or the previous run has
    /// already finished and been recorded. Proceed normally.
    Fire,
}

/// Decides what to do when a schedule is about to fire. For
/// `persistent_thread` schedules the caller passes the existing
/// task id (if any) and the schedule's `overlap_policy`; for
/// fresh-thread and throwaway modes the caller always passes
/// `None` because there is no concept of overlap — every fire is
/// independent.
pub fn decide_overlap(
    snapshot: &Snapshot,
    schedule: &Schedule,
    existing_task_id: Option<&str>,
) -> OverlapDecision {
    if schedule.mode != ScheduleMode::PersistentThread {
        return OverlapDecision::Fire;
    }
    let Some(task_id) = existing_task_id else {
        return OverlapDecision::Fire;
    };
    let Some(task) = snapshot.tasks.iter().find(|t| t.id == task_id) else {
        return OverlapDecision::Fire;
    };
    if task.status != "running" {
        return OverlapDecision::Fire;
    }
    match schedule.overlap_policy {
        OverlapPolicy::Skip => OverlapDecision::Skip,
        OverlapPolicy::Queue => OverlapDecision::Queue,
    }
}

/// Updates a schedule's bookkeeping after a successful fire.
/// `last_fire_at_ms` is recorded for the activity feed; the
/// consecutive-failure counter is reset to zero on success.
pub fn mark_schedule_succeeded(schedule: &mut Schedule, fired_at_ms: i64) {
    schedule.last_fire_at_ms = Some(fired_at_ms);
    schedule.consecutive_failures = 0;
    schedule.updated_at = crate::model::now();
}

/// Increments the failure counter on a failed fire and applies the
/// auto-pause rule. Returns `true` when the schedule has crossed
/// the threshold and should be disabled.
pub fn record_schedule_failure(
    schedule: &mut Schedule,
    fired_at_ms: i64,
) -> bool {
    schedule.last_fire_at_ms = Some(fired_at_ms);
    schedule.consecutive_failures = schedule.consecutive_failures.saturating_add(1);
    schedule.updated_at = crate::model::now();
    match schedule.max_consecutive_failures {
        Some(threshold) if schedule.consecutive_failures >= threshold => {
            schedule.enabled = false;
            true
        }
        _ => false,
    }
}

fn millis_to_datetime(ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ScheduleRunStatus;

    fn make_schedule(cron: &str, tz: &str) -> Schedule {
        Schedule {
            id: "sch-1".into(),
            title: "Test".into(),
            agent_id: "agent-1".into(),
            prompt: "ping".into(),
            frequency: cron.into(),
            tz: tz.into(),
            mode: ScheduleMode::NewThreadPerFire,
            overlap_policy: OverlapPolicy::Skip,
            max_consecutive_failures: None,
            enabled: true,
            last_fire_at_ms: None,
            consecutive_failures: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn validate_cron_accepts_standard_expressions() {
        assert!(validate_cron("*/15 * * * *").is_ok());
        assert!(validate_cron("0 9 * * 1-5").is_ok());
        assert!(validate_cron("30 8,18 * * 2,4").is_ok());
        assert!(validate_cron("0 0 1 1 *").is_ok());
    }

    #[test]
    fn validate_cron_rejects_bad_input() {
        assert!(validate_cron("").is_err());
        assert!(validate_cron("not a cron").is_err());
        assert!(validate_cron("* * * *").is_err()); // 4 fields
        assert!(validate_cron("* * * * * *").is_err()); // 6 fields
        assert!(validate_cron("99 * * * *").is_err()); // out of range
    }

    #[test]
    fn next_fire_after_finds_next_minute_boundary() {
        let s = make_schedule("*/5 * * * *", "UTC");
        let from = 1_700_000_000_000;
        let next = s.next_fire_after(from).expect("cron valid");
        assert!(next > from);
        // Next boundary is at most 5 minutes later.
        assert!(next - from <= 5 * 60 * 1000);
    }

    #[test]
    fn next_fire_after_handles_daily_9am() {
        let s = make_schedule("0 9 * * *", "UTC");
        let from = 1_700_000_000_000;
        let next = s.next_fire_after(from).expect("cron valid");
        assert!(next - from <= 24 * 60 * 60 * 1000);
        assert!(next - from > 0);
    }

    #[test]
    fn presets_translate_to_cron() {
        assert_eq!(SchedulePreset::Every15Minutes.cron(), "*/15 * * * *");
        assert_eq!(SchedulePreset::Daily9am.cron(), "0 9 * * *");
        assert_eq!(SchedulePreset::WeekdayMornings.cron(), "0 9 * * 1-5");
        assert_eq!(SchedulePreset::WeeklyMonday.cron(), "0 9 * * 1");
        assert_eq!(SchedulePreset::MonthlyFirst.cron(), "0 9 1 * *");
    }

    #[test]
    fn presets_parse_known_names() {
        assert!(matches!(
            SchedulePreset::parse("every_15_minutes"),
            Some(SchedulePreset::Every15Minutes)
        ));
        assert!(matches!(
            SchedulePreset::parse("daily_9am"),
            Some(SchedulePreset::Daily9am)
        ));
        assert!(SchedulePreset::parse("never_ever").is_none());
    }

    #[test]
    fn normalise_clears_queue_on_non_persistent_mode() {
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.mode = ScheduleMode::NewThreadPerFire;
        s.overlap_policy = OverlapPolicy::Queue;
        s.normalise().unwrap();
        assert_eq!(s.overlap_policy, OverlapPolicy::Skip);
    }

    #[test]
    fn normalise_rejects_empty_prompt() {
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.prompt.clear();
        assert!(s.normalise().is_err());
    }

    #[test]
    fn normalise_rejects_bad_tz() {
        let mut s = make_schedule("*/5 * * * *", "Atlantis/Lemuria");
        assert!(s.normalise().is_err());
    }

    #[test]
    fn normalise_accepts_host_tz() {
        let mut s = make_schedule("*/5 * * * *", "host");
        assert!(s.normalise().is_ok());
    }

    #[test]
    fn mode_round_trips_through_stored_string() {
        assert_eq!(
            ScheduleMode::PersistentThread.stored(),
            "persistent_thread"
        );
        assert_eq!(
            ScheduleMode::parse("throwaway").unwrap(),
            ScheduleMode::Throwaway
        );
    }

    #[test]
    fn run_status_round_trips() {
        assert_eq!(ScheduleRunStatus::Skipped.stored(), "skipped");
        assert_eq!(
            ScheduleRunStatus::parse("succeeded").unwrap(),
            ScheduleRunStatus::Succeeded
        );
    }

    #[test]
    fn recent_runs_for_schedule_returns_newest_first() {
        let runs = vec![
            ScheduleRun {
                id: "r1".into(),
                schedule_id: "s1".into(),
                fired_at_ms: 100,
                mode: ScheduleMode::NewThreadPerFire,
                task_id: None,
                status: ScheduleRunStatus::Succeeded,
                finished_at_ms: Some(110),
                summary: "old".into(),
                error: None,
                skipped_overlap: false,
            },
            ScheduleRun {
                id: "r2".into(),
                schedule_id: "s2".into(),
                fired_at_ms: 200,
                mode: ScheduleMode::NewThreadPerFire,
                task_id: None,
                status: ScheduleRunStatus::Succeeded,
                finished_at_ms: Some(210),
                summary: "other schedule".into(),
                error: None,
                skipped_overlap: false,
            },
            ScheduleRun {
                id: "r3".into(),
                schedule_id: "s1".into(),
                fired_at_ms: 300,
                mode: ScheduleMode::NewThreadPerFire,
                task_id: None,
                status: ScheduleRunStatus::Succeeded,
                finished_at_ms: Some(310),
                summary: "new".into(),
                error: None,
                skipped_overlap: false,
            },
        ];
        let result = recent_runs_for_schedule(&runs, "s1", 10);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, "r3");
        assert_eq!(result[1].id, "r1");
    }

    #[test]
    fn input_with_preset_resolves_to_cron() {
        let input = ScheduleInput {
            id: String::new(),
            title: "t".into(),
            agent_id: "a".into(),
            prompt: "p".into(),
            preset: Some("every_15_minutes".into()),
            frequency: None,
            tz: "host".into(),
            mode: ScheduleMode::PersistentThread,
            overlap_policy: None,
            max_consecutive_failures: None,
            enabled: true,
        };
        assert_eq!(input.resolve_frequency().unwrap(), "*/15 * * * *");
    }

    #[test]
    fn input_with_frequency_keeps_raw_cron() {
        let input = ScheduleInput {
            id: String::new(),
            title: "t".into(),
            agent_id: "a".into(),
            prompt: "p".into(),
            preset: None,
            frequency: Some("*/17 * * * *".into()),
            tz: "host".into(),
            mode: ScheduleMode::PersistentThread,
            overlap_policy: None,
            max_consecutive_failures: None,
            enabled: true,
        };
        assert_eq!(input.resolve_frequency().unwrap(), "*/17 * * * *");
    }

    #[test]
    fn input_without_either_rejects() {
        let input = ScheduleInput {
            id: String::new(),
            title: "t".into(),
            agent_id: "a".into(),
            prompt: "p".into(),
            preset: None,
            frequency: None,
            tz: "host".into(),
            mode: ScheduleMode::PersistentThread,
            overlap_policy: None,
            max_consecutive_failures: None,
            enabled: true,
        };
        assert!(input.resolve_frequency().is_err());
    }

    fn make_snapshot_with_task(task_id: &str, status: &str) -> Snapshot {
        use crate::model::Task;
        let mut snap = crate::model::default_snapshot();
        snap.tasks.push(Task {
            id: task_id.into(),
            agent_id: "agent-1".into(),
            title: "t".into(),
            native_session_id: None,
            archived: false,
            status: status.into(),
            created_at: 0,
            updated_at: 0,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            host_id: snap.hosts[0].id.clone(),
            cwd: "/tmp".into(),
            provider: "codex".into(),
            model: String::new(),
            sandbox: "read-only".into(),
            codex_home: None,
            acp: None,
            model_settings: None,
            archived_agent_name: None,
        });
        snap
    }

    #[test]
    fn overlap_decision_skip_when_running_and_policy_is_skip() {
        let snap = make_snapshot_with_task("task-1", "running");
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.mode = ScheduleMode::PersistentThread;
        s.overlap_policy = OverlapPolicy::Skip;
        let decision = decide_overlap(&snap, &s, Some("task-1"));
        assert_eq!(decision, OverlapDecision::Skip);
    }

    #[test]
    fn overlap_decision_queue_when_running_and_policy_is_queue() {
        let snap = make_snapshot_with_task("task-1", "running");
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.mode = ScheduleMode::PersistentThread;
        s.overlap_policy = OverlapPolicy::Queue;
        let decision = decide_overlap(&snap, &s, Some("task-1"));
        assert_eq!(decision, OverlapDecision::Queue);
    }

    #[test]
    fn overlap_decision_fire_when_previous_completed() {
        let snap = make_snapshot_with_task("task-1", "completed");
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.mode = ScheduleMode::PersistentThread;
        s.overlap_policy = OverlapPolicy::Skip;
        let decision = decide_overlap(&snap, &s, Some("task-1"));
        assert_eq!(decision, OverlapDecision::Fire);
    }

    #[test]
    fn overlap_decision_fire_for_fresh_thread_modes_regardless_of_policy() {
        let snap = make_snapshot_with_task("task-1", "running");
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.mode = ScheduleMode::NewThreadPerFire;
        s.overlap_policy = OverlapPolicy::Queue; // gets forced to Skip
        let decision = decide_overlap(&snap, &s, Some("task-1"));
        assert_eq!(decision, OverlapDecision::Fire);
    }

    #[test]
    fn record_failure_auto_pauses_at_threshold() {
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.max_consecutive_failures = Some(2);
        // First failure: count = 1, below threshold, not paused.
        assert!(!record_schedule_failure(&mut s, 1));
        assert!(s.enabled);
        // Second failure: count = 2, at threshold, paused. Returns true.
        assert!(record_schedule_failure(&mut s, 2));
        assert!(!s.enabled);
        assert_eq!(s.consecutive_failures, 2);
    }

    #[test]
    fn record_success_resets_failure_counter() {
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.consecutive_failures = 4;
        mark_schedule_succeeded(&mut s, 100);
        assert_eq!(s.consecutive_failures, 0);
        assert_eq!(s.last_fire_at_ms, Some(100));
    }

    #[test]
    fn auto_pause_disabled_when_threshold_is_none() {
        let mut s = make_schedule("*/5 * * * *", "UTC");
        s.max_consecutive_failures = None;
        for i in 0..20 {
            assert!(!record_schedule_failure(&mut s, i));
        }
        assert!(s.enabled);
        assert_eq!(s.consecutive_failures, 20);
    }
}
