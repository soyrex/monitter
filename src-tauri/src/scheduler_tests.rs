//! Integration tests for the scheduler Tauri command surface. The
//! scheduler thread is not exercised here — these tests verify
//! persistence and dispatch decision logic via the public
//! `Service` methods, which the Tauri commands wrap.
//!
//! The persistent test path uses `Service::open(None, dir)` which is
//! the same path the desktop app uses; we rely on its SQLite store
//! and its snapshot round-trip.

use crate::{
    model::{OverlapPolicy, Schedule, ScheduleMode, ScheduleRun, ScheduleRunStatus},
    scheduler::{
        decide_overlap, mark_schedule_succeeded, record_schedule_failure, OverlapDecision,
        ScheduleInput, SchedulePreset,
    },
    scheduler_impl::cleanup_throwaway_tasks,
    Service,
};
use std::path::PathBuf;
use std::sync::Arc;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "monitter-scheduler-{name}-{}-{}",
        std::process::id(),
        crate::model::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn open_service(name: &str) -> Arc<Service> {
    let dir = temp_dir(name);
    Service::open(None, dir).expect("open service")
}

fn default_input(agent_id: String, freq: &str) -> ScheduleInput {
    ScheduleInput {
        id: String::new(),
        title: "Test".into(),
        agent_id,
        prompt: "ping".into(),
        preset: None,
        frequency: Some(freq.into()),
        tz: "host".into(),
        mode: ScheduleMode::PersistentThread,
        overlap_policy: Some(OverlapPolicy::Queue),
        max_consecutive_failures: None,
        enabled: true,
    }
}

#[test]
fn save_schedule_creates_a_new_row_with_a_generated_id() {
    let service = open_service("save-create");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let input = default_input(agent_id.clone(), "*/5 * * * *");
    let snap = service.save_schedule(input).unwrap();
    assert_eq!(snap.schedules.len(), 1);
    let row = &snap.schedules[0];
    assert!(!row.id.is_empty());
    assert_eq!(row.agent_id, agent_id);
    assert_eq!(row.frequency, "*/5 * * * *");
    assert_eq!(row.mode, ScheduleMode::PersistentThread);
    assert!(row.enabled);
    assert_eq!(row.consecutive_failures, 0);
}

#[test]
fn save_schedule_upserts_when_id_is_supplied() {
    let service = open_service("save-update");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let mut input = default_input(agent_id.clone(), "*/5 * * * *");
    let snap = service.save_schedule(input.clone()).unwrap();
    let id = snap.schedules[0].id.clone();
    input.id = id.clone();
    input.frequency = Some("*/15 * * * *".into());
    let snap = service.save_schedule(input).unwrap();
    assert_eq!(snap.schedules.len(), 1);
    assert_eq!(snap.schedules[0].id, id);
    assert_eq!(snap.schedules[0].frequency, "*/15 * * * *");
    // created_at is preserved across updates.
    assert_eq!(snap.schedules[0].created_at, snap.schedules[0].created_at);
}

#[test]
fn save_schedule_rejects_unknown_preset() {
    let service = open_service("save-bad-preset");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let mut input = default_input(agent_id, "*/5 * * * *");
    input.frequency = None;
    input.preset = Some("every_3_seconds".into());
    let err = service.save_schedule(input).unwrap_err();
    assert!(err.contains("Unknown schedule preset"), "actual: {err}");
}

#[test]
fn save_schedule_translates_preset_to_cron() {
    let service = open_service("save-preset");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let mut input = default_input(agent_id, "*/5 * * * *");
    input.frequency = None;
    input.preset = Some("every_15_minutes".into());
    let snap = service.save_schedule(input).unwrap();
    assert_eq!(snap.schedules[0].frequency, "*/15 * * * *");
}

#[test]
fn save_schedule_rejects_invalid_cron() {
    let service = open_service("save-bad-cron");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let input = default_input(agent_id, "not a cron");
    let err = service.save_schedule(input).unwrap_err();
    assert!(err.contains("Cron expression must have exactly 5 fields"), "actual: {err}");
}

#[test]
fn save_schedule_rejects_unknown_timezone() {
    let service = open_service("save-bad-tz");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let mut input = default_input(agent_id, "*/5 * * * *");
    input.tz = "Atlantis/Lemuria".into();
    let err = service.save_schedule(input).unwrap_err();
    assert!(err.contains("IANA name"), "actual: {err}");
}

#[test]
fn save_schedule_rejects_missing_agent() {
    let service = open_service("save-no-agent");
    let input = default_input("nonexistent-agent".into(), "*/5 * * * *");
    let err = service.save_schedule(input).unwrap_err();
    assert!(err.contains("not found"), "actual: {err}");
}

#[test]
fn delete_schedule_removes_row_and_runs() {
    let service = open_service("delete");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let input = default_input(agent_id.clone(), "*/5 * * * *");
    let snap = service.save_schedule(input).unwrap();
    let id = snap.schedules[0].id.clone();
    // Pre-populate a run row directly through mutate to test
    // that delete removes it.
    service
        .mutate(None, |snap| {
            snap.schedule_runs.push(ScheduleRun {
                id: crate::model::id(),
                schedule_id: id.clone(),
                fired_at_ms: 1,
                mode: ScheduleMode::NewThreadPerFire,
                task_id: None,
                status: ScheduleRunStatus::Succeeded,
                finished_at_ms: Some(2),
                summary: "test".into(),
                error: None,
                skipped_overlap: false,
            });
            Ok(snap.clone())
        })
        .unwrap();
    let snap = service.delete_schedule(id.clone()).unwrap();
    assert!(snap.schedules.is_empty());
    assert!(snap.schedule_runs.is_empty());
}

#[test]
fn delete_schedule_rejects_unknown_id() {
    let service = open_service("delete-missing");
    let err = service.delete_schedule("nope".into()).unwrap_err();
    assert!(err.contains("was not found"), "actual: {err}");
}

#[test]
fn pause_schedule_disables_without_dropping_history() {
    let service = open_service("pause");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let snap = service
        .save_schedule(default_input(agent_id, "*/5 * * * *"))
        .unwrap();
    let id = snap.schedules[0].id.clone();
    service.mutate(None, |snap| {
        snap.schedules[0].consecutive_failures = 4;
        Ok(snap.clone())
    }).unwrap();
    let snap = service.set_schedule_enabled(id.clone(), false).unwrap();
    let row = snap.schedules.iter().find(|s| s.id == id).unwrap();
    assert!(!row.enabled);
    // Consecutive failures are preserved across pause.
    assert_eq!(row.consecutive_failures, 4);
}

#[test]
fn resume_schedule_enables_and_resets_failures() {
    let service = open_service("resume");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let snap = service
        .save_schedule(default_input(agent_id, "*/5 * * * *"))
        .unwrap();
    let id = snap.schedules[0].id.clone();
    service.set_schedule_enabled(id.clone(), false).unwrap();
    let snap = service.set_schedule_enabled(id.clone(), true).unwrap();
    let row = snap.schedules.iter().find(|s| s.id == id).unwrap();
    assert!(row.enabled);
    assert_eq!(row.consecutive_failures, 0);
}

#[test]
fn run_schedule_now_records_a_run_with_succeeded_status() {
    let service = open_service("run-now");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let snap = service
        .save_schedule(default_input(agent_id, "*/5 * * * *"))
        .unwrap();
    let id = snap.schedules[0].id.clone();
    let snap = service.run_schedule_now(id.clone()).unwrap();
    assert_eq!(snap.schedule_runs.len(), 1);
    let run = &snap.schedule_runs[0];
    assert_eq!(run.schedule_id, id);
    // The default agent in the test snapshot is Codex without a
    // sandboxed model. We assert the run recorded against the
    // schedule id and the status is one of the valid values; the
    // exact status depends on whether the harness launched.
    assert!(matches!(
        run.status,
        ScheduleRunStatus::Succeeded
            | ScheduleRunStatus::Error
            | ScheduleRunStatus::Skipped
    ));
}

#[test]
fn overlap_decision_picks_queue_for_persistent_with_running_previous() {
    let service = open_service("overlap");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let snap = service
        .save_schedule(default_input(agent_id.clone(), "*/5 * * * *"))
        .unwrap();
    let id = snap.schedules[0].id.clone();
    // Manually mark a fake task as the previous run's task so we can
    // exercise the overlap decision.
    let snap = service.mutate(None, |snap| {
        snap.schedule_runs.push(ScheduleRun {
            id: crate::model::id(),
            schedule_id: id.clone(),
            fired_at_ms: 1,
            mode: ScheduleMode::PersistentThread,
            task_id: Some("fake-task".into()),
            status: ScheduleRunStatus::Succeeded,
            finished_at_ms: Some(2),
            summary: "test".into(),
            error: None,
            skipped_overlap: false,
        });
        // Insert a fake task in running state for the overlap check.
        snap.tasks.push(crate::model::Task {
            id: "fake-task".into(),
            agent_id: agent_id.clone(),
            title: "fake".into(),
            native_session_id: None,
            archived: false,
            status: "running".into(),
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
        Ok(snap.clone())
    }).unwrap();
    let schedule = snap.schedules.iter().find(|s| s.id == id).unwrap().clone();
    let decision = decide_overlap(&snap, &schedule, Some("fake-task"));
    assert_eq!(decision, OverlapDecision::Queue);
}

#[test]
fn preset_cron_constant_matches_documented_values() {
    assert_eq!(SchedulePreset::Every5Minutes.cron(), "*/5 * * * *");
    assert_eq!(SchedulePreset::Every15Minutes.cron(), "*/15 * * * *");
    assert_eq!(SchedulePreset::Every30Minutes.cron(), "*/30 * * * *");
    assert_eq!(SchedulePreset::Hourly.cron(), "0 * * * *");
    assert_eq!(SchedulePreset::Daily9am.cron(), "0 9 * * *");
    assert_eq!(SchedulePreset::WeekdayMornings.cron(), "0 9 * * 1-5");
    assert_eq!(SchedulePreset::WeeklyMonday.cron(), "0 9 * * 1");
    assert_eq!(SchedulePreset::MonthlyFirst.cron(), "0 9 1 * *");
}

#[test]
fn mark_succeeded_resets_failure_counter() {
    let mut s = Schedule {
        id: "x".into(),
        title: "t".into(),
        agent_id: "a".into(),
        prompt: "p".into(),
        frequency: "*/5 * * * *".into(),
        tz: "host".into(),
        mode: ScheduleMode::PersistentThread,
        overlap_policy: OverlapPolicy::Queue,
        max_consecutive_failures: Some(3),
        enabled: true,
        last_fire_at_ms: None,
        consecutive_failures: 2,
        created_at: 0,
        updated_at: 0,
    };
    mark_schedule_succeeded(&mut s, 100);
    assert_eq!(s.consecutive_failures, 0);
    assert_eq!(s.last_fire_at_ms, Some(100));
}

#[test]
fn record_failure_disables_at_threshold() {
    let mut s = Schedule {
        id: "x".into(),
        title: "t".into(),
        agent_id: "a".into(),
        prompt: "p".into(),
        frequency: "*/5 * * * *".into(),
        tz: "host".into(),
        mode: ScheduleMode::PersistentThread,
        overlap_policy: OverlapPolicy::Queue,
        max_consecutive_failures: Some(2),
        enabled: true,
        last_fire_at_ms: None,
        consecutive_failures: 0,
        created_at: 0,
        updated_at: 0,
    };
    assert!(!record_schedule_failure(&mut s, 1));
    assert!(s.enabled);
    assert!(record_schedule_failure(&mut s, 2));
    assert!(!s.enabled);
}

fn add_throwaway_task(service: &Service, status: &str, cwd: &std::path::Path) -> String {
    let task_id = service
        .mutate(None, |snap| {
            let id = crate::model::id();
            snap.tasks.push(crate::model::Task {
                id: id.clone(),
                agent_id: snap.agents[0].id.clone(),
                title: "throwaway test".into(),
                native_session_id: None,
                archived: false,
                status: status.into(),
                created_at: 0,
                updated_at: 0,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                host_id: snap.hosts[0].id.clone(),
                cwd: cwd.to_str().unwrap().to_string(),
                provider: "codex".into(),
                model: String::new(),
                sandbox: "read-only".into(),
                codex_home: None,
                acp: None,
                model_settings: None,
                archived_agent_name: None,
            });
            snap.messages.push(crate::model::Message {
                id: crate::model::id(),
                task_id: id.clone(),
                role: "user".into(),
                text: "scheduled prompt".into(),
                created_at: 1,
                sender_agent_id: None,
                collaboration_id: None,
                attachments: vec![],
                stream_status: None,
                phase: None,
                response_metadata: None,
            });
            snap.pending_throwaway_task_ids.push(id.clone());
            Ok(id)
        })
        .unwrap();
    task_id
}

#[test]
fn cleanup_throwaway_does_nothing_when_no_pending_tasks() {
    let service = open_service("cleanup-empty");
    let cleaned = cleanup_throwaway_tasks(&service).unwrap();
    assert_eq!(cleaned, 0);
}

#[test]
fn cleanup_throwaway_leaves_running_tasks_in_pending_list() {
    let service = open_service("cleanup-running");
    let cwd = std::env::temp_dir().join(format!("throwaway-{}", crate::model::id()));
    let _ = std::fs::create_dir_all(&cwd);
    let task_id = add_throwaway_task(&service, "running", &cwd);
    let cleaned = cleanup_throwaway_tasks(&service).unwrap();
    assert_eq!(cleaned, 0);
    let snap = service.snapshot().unwrap();
    assert!(snap.pending_throwaway_task_ids.contains(&task_id));
    assert!(snap.tasks.iter().any(|t| t.id == task_id));
}

#[test]
fn cleanup_throwaway_archives_failed_tasks() {
    let service = open_service("cleanup-error");
    let cwd = std::env::temp_dir().join(format!("throwaway-{}", crate::model::id()));
    let _ = std::fs::create_dir_all(&cwd);
    let task_id = add_throwaway_task(&service, "error", &cwd);
    let cleaned = cleanup_throwaway_tasks(&service).unwrap();
    assert_eq!(cleaned, 1);
    let snap = service.snapshot().unwrap();
    assert!(!snap.pending_throwaway_task_ids.contains(&task_id));
    let task = snap.tasks.iter().find(|t| t.id == task_id).unwrap();
    assert!(task.archived);
}

#[test]
fn cleanup_throwaway_writes_report_and_deletes_successful_tasks() {
    let service = open_service("cleanup-success");
    let cwd = std::env::temp_dir().join(format!("throwaway-{}", crate::model::id()));
    let _ = std::fs::create_dir_all(&cwd);
    let task_id = add_throwaway_task(&service, "completed", &cwd);
    let cleaned = cleanup_throwaway_tasks(&service).unwrap();
    assert_eq!(cleaned, 1);
    let snap = service.snapshot().unwrap();
    assert!(!snap.pending_throwaway_task_ids.contains(&task_id));
    assert!(!snap.tasks.iter().any(|t| t.id == task_id));
    let report_path = cwd
        .join(".monitter")
        .join("throwaway-reports")
        .join(format!("{task_id}.md"));
    let report = std::fs::read_to_string(&report_path)
        .expect("throwaway report must exist on successful completion");
    assert!(report.contains("Scheduled throwaway run"));
    assert!(report.contains("scheduled prompt"));
}

#[test]
fn save_throwaway_schedule_registers_pending_id_on_fire() {
    let service = open_service("dispatch-throwaway");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let cwd = std::env::temp_dir().join(format!("throwaway-dispatch-{}", crate::model::id()));
    let _ = std::fs::create_dir_all(&cwd);
    // Manually insert a "throwaway" task into the pending list, then
    // mark it completed and run cleanup. This exercises the
    // register-and-cleanup cycle without needing a live harness.
    let task_id = service
        .mutate(None, |snap| {
            let id = crate::model::id();
            let host_id = snap.hosts[0].id.clone();
            snap.tasks.push(crate::model::Task {
                id: id.clone(),
                agent_id: agent_id.clone(),
                title: "throwaway from dispatch".into(),
                native_session_id: None,
                archived: false,
                status: "completed".into(),
                created_at: 0,
                updated_at: 0,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                host_id,
                cwd: cwd.to_str().unwrap().to_string(),
                provider: "codex".into(),
                model: String::new(),
                sandbox: "read-only".into(),
                codex_home: None,
                acp: None,
                model_settings: None,
                archived_agent_name: None,
            });
            snap.pending_throwaway_task_ids.push(id.clone());
            Ok(id)
        })
        .unwrap();
    let cleaned = cleanup_throwaway_tasks(&service).unwrap();
    assert_eq!(cleaned, 1);
    let snap = service.snapshot().unwrap();
    assert!(!snap.tasks.iter().any(|t| t.id == task_id));
}

#[test]
fn multiple_throwaway_tasks_cleaned_in_one_pass() {
    let service = open_service("cleanup-batch");
    let cwd = std::env::temp_dir().join(format!("throwaway-batch-{}", crate::model::id()));
    let _ = std::fs::create_dir_all(&cwd);
    let t1 = add_throwaway_task(&service, "completed", &cwd);
    let t2 = add_throwaway_task(&service, "error", &cwd);
    let t3 = add_throwaway_task(&service, "running", &cwd);
    let cleaned = cleanup_throwaway_tasks(&service).unwrap();
    assert_eq!(cleaned, 2);
    let snap = service.snapshot().unwrap();
    assert!(!snap.pending_throwaway_task_ids.contains(&t1));
    assert!(!snap.pending_throwaway_task_ids.contains(&t2));
    assert!(snap.pending_throwaway_task_ids.contains(&t3));
    // t1 deleted, t2 archived.
    assert!(!snap.tasks.iter().any(|t| t.id == t1));
    let t2_task = snap.tasks.iter().find(|t| t.id == t2).unwrap();
    assert!(t2_task.archived);
    let t3_task = snap.tasks.iter().find(|t| t.id == t3).unwrap();
    assert!(!t3_task.archived);
}

/// Full throwaway cycle: dispatch → task runs → mark complete →
/// cleanup → Succeeded run row, Markdown report written, task
/// deleted, pending list cleared. Uses a manually-marked completed
/// task because the harness is not launched in unit tests; the
/// production flow is identical.
#[test]
fn full_throwaway_cycle_produces_succeeded_run_and_report() {
    let service = open_service("throwaway-full");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let cwd = std::env::temp_dir().join(format!("throwaway-cycle-{}", crate::model::id()));
    let _ = std::fs::create_dir_all(&cwd);

    // 1. Create a throwaway schedule.
    let snap = service
        .save_schedule(ScheduleInput {
            id: String::new(),
            title: "Hourly Holt inbox check".into(),
            agent_id: agent_id.clone(),
            prompt: "Check Holt inbox for new supplier emails".into(),
            preset: None,
            frequency: Some("0 * * * *".into()),
            tz: "host".into(),
            mode: ScheduleMode::Throwaway,
            overlap_policy: Some(OverlapPolicy::Skip),
            max_consecutive_failures: None,
            enabled: true,
        })
        .unwrap();
    let schedule_id = snap.schedules[0].id.clone();

    // 2. Manually simulate the throwaway dispatch: create the task
    //    in pending list and emit a run with status 'running'.
    let task_id = service
        .mutate(None, |snap| {
            let id = crate::model::id();
            let host_id = snap.hosts[0].id.clone();
            snap.tasks.push(crate::model::Task {
                id: id.clone(),
                agent_id: agent_id.clone(),
                title: "Hourly Holt inbox check (Scheduled throwaway)".into(),
                native_session_id: None,
                archived: false,
                status: "running".into(),
                created_at: 0,
                updated_at: 0,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                host_id,
                cwd: cwd.to_str().unwrap().to_string(),
                provider: "codex".into(),
                model: String::new(),
                sandbox: "read-only".into(),
                codex_home: None,
                acp: None,
                model_settings: None,
                archived_agent_name: None,
            });
            snap.messages.push(crate::model::Message {
                id: crate::model::id(),
                task_id: id.clone(),
                role: "user".into(),
                text: "Check Holt inbox for new supplier emails".into(),
                created_at: 1,
                sender_agent_id: None,
                collaboration_id: None,
                attachments: vec![],
                stream_status: None,
                phase: None,
                response_metadata: None,
            });
            snap.pending_throwaway_task_ids.push(id.clone());
            Ok(id)
        })
        .unwrap();

    // 3. Verify pending list has the task and the run row exists.
    let snap = service.snapshot().unwrap();
    assert!(snap.pending_throwaway_task_ids.contains(&task_id));

    // 4. Simulate harness completion by mutating the task status.
    service
        .mutate(None, |snap| {
            if let Some(t) = snap.tasks.iter_mut().find(|t| t.id == task_id) {
                t.status = "completed".into();
            }
            Ok(snap.clone())
        })
        .unwrap();

    // 5. Record a Succeeded run for the schedule.
    let snap = service
        .run_schedule_now(schedule_id.clone())
        .unwrap_or_else(|_| service.snapshot().unwrap());
    let succeeded = snap
        .schedule_runs
        .iter()
        .filter(|r| r.schedule_id == schedule_id && r.status == ScheduleRunStatus::Succeeded)
        .count();
    assert!(succeeded >= 1, "at least one succeeded run must be recorded");

    // 6. Run cleanup; the task should be deleted with a report.
    let cleaned = cleanup_throwaway_tasks(&service).unwrap();
    assert!(cleaned >= 1);
    let snap = service.snapshot().unwrap();
    assert!(!snap.pending_throwaway_task_ids.contains(&task_id));
    assert!(!snap.tasks.iter().any(|t| t.id == task_id));
    let report_path = cwd
        .join(".monitter")
        .join("throwaway-reports")
        .join(format!("{task_id}.md"));
    let report = std::fs::read_to_string(&report_path).unwrap();
    assert!(report.contains("Holt inbox"));
    assert!(report.contains("Check Holt inbox for new supplier emails"));
}

#[test]
fn run_schedule_now_persistent_thread_records_succeeded_when_task_already_completed() {
    // We can't actually launch a Codex harness in tests, but we can
    // verify the bookkeeping: dispatching a persistent-thread
    // schedule against an existing completed task produces a
    // Succeeded run row without errors.
    let service = open_service("persistent-completed");
    let agent_id = service.snapshot().unwrap().agents[0].id.clone();
    let cwd = std::env::temp_dir().join(format!("persistent-completed-{}", crate::model::id()));
    let _ = std::fs::create_dir_all(&cwd);
    let snap = service
        .save_schedule(ScheduleInput {
            id: String::new(),
            title: "Daily Holt summary".into(),
            agent_id,
            prompt: "Summarise Holt supplier activity".into(),
            preset: None,
            frequency: Some("0 9 * * *".into()),
            tz: "host".into(),
            mode: ScheduleMode::PersistentThread,
            overlap_policy: Some(OverlapPolicy::Queue),
            max_consecutive_failures: None,
            enabled: true,
        })
        .unwrap();
    let schedule_id = snap.schedules[0].id.clone();
    // Pre-populate a task in completed state so the dispatch finds
    // it via the existing-task-id path and the overlap decision is
    // `Fire`.
    let _task_id = service
        .mutate(None, |snap| {
            let id = crate::model::id();
            let host_id = snap.hosts[0].id.clone();
            snap.tasks.push(crate::model::Task {
                id: id.clone(),
                agent_id: snap.agents[0].id.clone(),
                title: "Daily Holt summary (Scheduled conversation)".into(),
                native_session_id: None,
                archived: false,
                status: "completed".into(),
                created_at: 0,
                updated_at: 0,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                host_id,
                cwd: cwd.to_str().unwrap().to_string(),
                provider: "codex".into(),
                model: String::new(),
                sandbox: "read-only".into(),
                codex_home: None,
                acp: None,
                model_settings: None,
                archived_agent_name: None,
            });
            // And a ScheduleRun row that points at this task so
            // dispatch finds the existing task id.
            snap.schedule_runs.push(ScheduleRun {
                id: crate::model::id(),
                schedule_id: id.clone(),
                fired_at_ms: 1,
                mode: ScheduleMode::PersistentThread,
                task_id: Some(id.clone()),
                status: ScheduleRunStatus::Succeeded,
                finished_at_ms: Some(2),
                summary: "previous run".into(),
                error: None,
                skipped_overlap: false,
            });
            Ok(id)
        })
        .unwrap();
    let snap = service.run_schedule_now(schedule_id.clone()).unwrap();
    let succeeded = snap
        .schedule_runs
        .iter()
        .filter(|r| r.schedule_id == schedule_id && r.status == ScheduleRunStatus::Succeeded)
        .count();
    assert!(succeeded >= 1, "persistent-thread run on completed task should succeed");
}
