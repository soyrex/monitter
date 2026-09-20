//! `Service` impl methods for the scheduler. The pure data and
//! decision logic lives in `scheduler.rs`; everything that touches
//! `Service`'s state-mutation API, the runner, or the message dispatch
//! pipeline lives here so the dependency direction stays clear.

use crate::{
    model::{
        CreateTaskInput, RunEvent, Schedule, ScheduleMode, ScheduleRun, ScheduleRunStatus,
        Snapshot,
    },
    scheduler::{
        decide_overlap, mark_schedule_succeeded, record_schedule_failure,
        resolve_schedule_agent, OverlapDecision, ScheduleInput,
    },
    Service,
};
use std::collections::HashMap;
use std::sync::Arc;

impl Service {
    /// `list_schedules` handler. Returns the full snapshot so the
    /// frontend can render without a follow-up read.
    pub(crate) fn list_schedules(self: &Arc<Self>) -> Result<Snapshot, String> {
        self.snapshot()
    }

    /// `save_schedule` handler. Validates input, normalises the
    /// schedule (preset → cron, queue→skip on non-persistent, etc.),
    /// and either creates or updates the schedule row.
    pub(crate) fn save_schedule(
        self: &Arc<Self>,
        input: ScheduleInput,
    ) -> Result<Snapshot, String> {
        let mut schedule = input.into_schedule()?;
        // Verify the agent exists and is non-internal.
        {
            let data = self
                .data
                .lock()
                .map_err(|_| "Monitter state lock failed.".to_string())?;
            resolve_schedule_agent(&data.snapshot, &schedule.agent_id)?;
        }
        let now = crate::model::now();
        self.mutate(None, |snapshot| {
            if schedule.id.is_empty() {
                schedule.id = crate::model::id();
                schedule.created_at = now;
                schedule.updated_at = now;
                schedule.consecutive_failures = 0;
                schedule.last_fire_at_ms = None;
                snapshot.schedules.push(schedule.clone());
            } else {
                let existing = snapshot
                    .schedules
                    .iter_mut()
                    .find(|row| row.id == schedule.id)
                    .ok_or_else(|| {
                        format!("Schedule '{}' was not found.", schedule.id)
                    })?;
                // Preserve the durable fields the user does not edit.
                schedule.created_at = existing.created_at;
                schedule.last_fire_at_ms = existing.last_fire_at_ms;
                schedule.consecutive_failures = existing.consecutive_failures;
                schedule.updated_at = now;
                *existing = schedule.clone();
            }
            Ok(snapshot.clone())
        })
    }

    /// `delete_schedule` handler. Removes the schedule row and any
    /// historical runs. Does not touch tasks created by past fires.
    pub(crate) fn delete_schedule(self: &Arc<Self>, id: String) -> Result<Snapshot, String> {
        self.mutate(None, |snapshot| {
            let before = snapshot.schedules.len();
            snapshot.schedules.retain(|row| row.id != id);
            if snapshot.schedules.len() == before {
                return Err(format!("Schedule '{id}' was not found."));
            }
            snapshot.schedule_runs.retain(|run| run.schedule_id != id);
            Ok(snapshot.clone())
        })
    }

    /// `pause_schedule` and `resume_schedule` share a single helper
    /// that flips the `enabled` flag.
    pub(crate) fn set_schedule_enabled(
        self: &Arc<Self>,
        id: String,
        enabled: bool,
    ) -> Result<Snapshot, String> {
        self.mutate(None, |snapshot| {
            let schedule = snapshot
                .schedules
                .iter_mut()
                .find(|row| row.id == id)
                .ok_or_else(|| format!("Schedule '{id}' was not found."))?;
            schedule.enabled = enabled;
            schedule.updated_at = crate::model::now();
            if enabled {
                schedule.consecutive_failures = 0;
            }
            Ok(snapshot.clone())
        })
    }

    /// `run_schedule_now` handler. Dispatches the schedule
    /// synchronously, records a run, and returns the snapshot. Used
    /// by the chat-MCP tool and by tests.
    pub(crate) fn run_schedule_now(self: &Arc<Self>, id: String) -> Result<Snapshot, String> {
        let (schedule, snapshot) = {
            let data = self
                .data
                .lock()
                .map_err(|_| "Monitter state lock failed.".to_string())?;
            let schedule = data
                .snapshot
                .schedules
                .iter()
                .find(|row| row.id == id)
                .cloned()
                .ok_or_else(|| format!("Schedule '{id}' was not found."))?;
            (schedule, data.snapshot.clone())
        };
        self.dispatch_schedule_fire(&schedule, &snapshot, true)
    }

    /// Dispatches a single fire of `schedule`. `forced` is `true`
    /// when called from `run_schedule_now`; the scheduler loop calls
    /// with `forced: false` so user-driven fires and timer fires
    /// remain distinguishable in the run log.
    pub(crate) fn dispatch_schedule_fire(
        self: &Arc<Self>,
        schedule: &Schedule,
        snapshot: &Snapshot,
        forced: bool,
    ) -> Result<Snapshot, String> {
        let fired_at_ms = crate::model::now();
        // Decide what to do about overlap. Persistent-thread
        // schedules may queue or skip; other modes always fire.
        let existing_task_id = snapshot
            .schedule_runs
            .iter()
            .rev()
            .find(|run| {
                run.schedule_id == schedule.id && run.task_id.is_some()
            })
            .and_then(|run| run.task_id.clone());
        let decision = decide_overlap(snapshot, schedule, existing_task_id.as_deref());
        if decision == OverlapDecision::Skip {
            return self.record_run(
                schedule,
                fired_at_ms,
                None,
                ScheduleRunStatus::Skipped,
                "Tick arrived while previous fire was still running; overlap policy is skip."
                    .into(),
                None,
                true,
            );
        }
        // The actual dispatch. Each branch resolves a task id, sends
        // the prompt, then writes a run row.
        let outcome = match schedule.mode {
            ScheduleMode::PersistentThread => {
                self.dispatch_persistent_thread(schedule, existing_task_id.as_deref(), forced)
            }
            ScheduleMode::NewThreadPerFire => self.dispatch_new_thread_per_fire(schedule, forced),
            ScheduleMode::Throwaway => self.dispatch_throwaway(schedule, forced),
        };
        match outcome {
            Ok(DispatchOutcome { task_id, summary, error }) => {
                let status = if error.is_some() {
                    ScheduleRunStatus::Error
                } else {
                    ScheduleRunStatus::Succeeded
                };
                self.record_run(
                    schedule,
                    fired_at_ms,
                    Some(task_id),
                    status,
                    summary,
                    error,
                    false,
                )
            }
            Err(error) => self.record_run(
                schedule,
                fired_at_ms,
                None,
                ScheduleRunStatus::Error,
                format!("Dispatch failed: {error}"),
                Some(error.clone()),
                false,
            ),
        }
    }

    fn dispatch_persistent_thread(
        self: &Arc<Self>,
        schedule: &Schedule,
        existing_task_id: Option<&str>,
        _forced: bool,
    ) -> Result<DispatchOutcome, String> {
        let task_id = match existing_task_id {
            Some(id) => id.to_string(),
            None => self
                .create_schedule_task(schedule, "Scheduled conversation")?
                .id,
        };
        self.send_fast(task_id.clone(), schedule.prompt.clone(), Vec::new())?;
        Ok(DispatchOutcome {
            task_id,
            summary: format!(
                "Sent scheduled prompt to persistent task for '{}'.",
                schedule.title
            ),
            error: None,
        })
    }

    fn dispatch_new_thread_per_fire(
        self: &Arc<Self>,
        schedule: &Schedule,
        _forced: bool,
    ) -> Result<DispatchOutcome, String> {
        let task = self.create_schedule_task(schedule, "Scheduled run")?;
        self.send_fast(task.id.clone(), schedule.prompt.clone(), Vec::new())?;
        Ok(DispatchOutcome {
            task_id: task.id,
            summary: format!(
                "Created task '{}' and sent scheduled prompt.",
                schedule.title
            ),
            error: None,
        })
    }

    fn dispatch_throwaway(
        self: &Arc<Self>,
        schedule: &Schedule,
        _forced: bool,
    ) -> Result<DispatchOutcome, String> {
        let task = self.create_schedule_task(schedule, "Scheduled throwaway")?;
        // Register the task id with the cleanup pass so it gets
        // archived (on failure) or archived + deleted with a
        // Markdown report (on success) once the task finishes.
        self.mutate(None, |snap| {
            if !snap.pending_throwaway_task_ids.contains(&task.id) {
                snap.pending_throwaway_task_ids.push(task.id.clone());
            }
            Ok(snap.clone())
        })?;
        self.send_fast(task.id.clone(), schedule.prompt.clone(), Vec::new())?;
        Ok(DispatchOutcome {
            task_id: task.id,
            summary: format!(
                "Created throwaway task '{}' and sent scheduled prompt.",
                schedule.title
            ),
            error: None,
        })
    }

    /// Creates a task owned by the schedule's agent. Centralised so
    /// future per-mode overrides land in one place.
    fn create_schedule_task(
        self: &Arc<Self>,
        schedule: &Schedule,
        title_suffix: &str,
    ) -> Result<crate::model::Task, String> {
        let input = CreateTaskInput {
            agent_id: schedule.agent_id.clone(),
            title: format!("{} ({})", schedule.title, title_suffix),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: None,
            cwd: None,
            model_settings: None,
            sandbox: None,
        };
        self.create_task(input)
    }

    /// Persists a `ScheduleRun` row, updates the schedule's
    /// bookkeeping (last fire, consecutive failures, auto-pause), and
    /// appends a `RunEvent` of kind `schedule` so the activity feed
    /// surfaces the fire.
    fn record_run(
        self: &Arc<Self>,
        schedule: &Schedule,
        fired_at_ms: i64,
        task_id: Option<String>,
        status: ScheduleRunStatus,
        summary: String,
        error: Option<String>,
        skipped_overlap: bool,
    ) -> Result<Snapshot, String> {
        let run = ScheduleRun {
            id: crate::model::id(),
            schedule_id: schedule.id.clone(),
            fired_at_ms,
            mode: schedule.mode,
            task_id: task_id.clone(),
            status,
            finished_at_ms: Some(crate::model::now()),
            summary: summary.clone(),
            error: error.clone(),
            skipped_overlap,
        };
        let event = RunEvent {
            id: crate::model::id(),
            task_id: task_id.clone().unwrap_or_default(),
            kind: "schedule".into(),
            title: format!("Scheduled: {}", schedule.title),
            detail: std::sync::Arc::from(build_run_detail(&run, &summary, error.as_deref())),
            created_at: fired_at_ms,
        };
        self.mutate_data(None, |data| {
            let Some(schedule_row) = data
                .snapshot
                .schedules
                .iter_mut()
                .find(|row| row.id == schedule.id)
                .cloned()
            else {
                return Err(format!(
                    "Schedule '{}' disappeared during dispatch.",
                    schedule.id
                ));
            };
            // Per-run bookkeeping.
            let mut updated = schedule_row.clone();
            match status {
                ScheduleRunStatus::Succeeded => {
                    mark_schedule_succeeded(&mut updated, fired_at_ms);
                }
                ScheduleRunStatus::Error => {
                    record_schedule_failure(&mut updated, fired_at_ms);
                }
                _ => {
                    updated.last_fire_at_ms = Some(fired_at_ms);
                    updated.updated_at = crate::model::now();
                }
            }
            if let Some(slot) = data
                .snapshot
                .schedules
                .iter_mut()
                .find(|row| row.id == schedule.id)
            {
                *slot = updated;
            }
            data.snapshot.schedule_runs.push(run);
            // Bound the run log so the snapshot doesn't grow
            // unboundedly. We keep the most recent 200 runs per
            // schedule and the most recent 500 globally.
            data.snapshot
                .schedule_runs
                .sort_by_key(|run| run.fired_at_ms);
            if data.snapshot.schedule_runs.len() > 500 {
                let excess = data.snapshot.schedule_runs.len() - 500;
                data.snapshot.schedule_runs.drain(0..excess);
            }
            let _ = truncate_per_schedule(&mut data.snapshot.schedule_runs, 200);
            data.snapshot.events.push(std::sync::Arc::new(event));
            Ok(data.snapshot.clone())
        })
    }

    /// Spawns the background scheduler thread. Called once from
    /// `Service::start_idle_collector` neighbour.
    pub(crate) fn start_scheduler(self: &Arc<Self>) {
        if self.scheduler_started.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || loop {
            std::thread::sleep(crate::scheduler::SCHEDULER_TICK);
            let Some(service) = weak.upgrade() else {
                break;
            };
            if service.stopping.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            service.scheduler_tick();
        });
    }

    /// One iteration of the scheduler loop: pick every enabled
    /// schedule whose `next_fire_after(now)` is in the past (or now)
    /// and dispatch it. Catches up at most one missed tick per
    /// schedule per loop turn so a long downtime does not produce a
    /// flood.
    fn scheduler_tick(self: &Arc<Self>) {
        // First, clean up any throwaway tasks that finished since the
        // last tick. Errors here are best-effort: they don't block
        // dispatch.
        let _ = cleanup_throwaway_tasks(self);
        let snapshot = match self.snapshot() {
            Ok(snap) => snap,
            Err(_) => return,
        };
        let now = crate::model::now();
        let mut fired: HashMap<String, i64> = HashMap::new();
        for schedule in snapshot
            .schedules
            .iter()
            .filter(|row| row.enabled)
        {
            let anchor = match schedule.last_fire_at_ms {
                Some(prev) => prev,
                None => now,
            };
            let Some(next) = schedule.next_fire_after(anchor) else {
                continue;
            };
            if next > now {
                continue;
            }
            // Already fired this wall-clock minute on a previous tick?
            if let Some(prev) = fired.get(&schedule.id) {
                if *prev >= next {
                    continue;
                }
            }
            if self.dispatch_schedule_fire(schedule, &snapshot, false).is_ok() {
                fired.insert(schedule.id.clone(), next);
            }
        }
    }
}

struct DispatchOutcome {
    task_id: String,
    summary: String,
    error: Option<String>,
}

fn build_run_detail(run: &ScheduleRun, summary: &str, error: Option<&str>) -> String {
    let mut parts = Vec::new();
    parts.push(format!("status: {}", run.status.stored()));
    if let Some(task_id) = run.task_id.as_deref() {
        parts.push(format!("task: {task_id}"));
    }
    parts.push(format!("mode: {}", run.mode.stored()));
    parts.push(format!("summary: {summary}"));
    if let Some(error) = error {
        parts.push(format!("error: {error}"));
    }
    if run.skipped_overlap {
        parts.push("skipped_overlap: true".into());
    }
    parts.join("\n")
}

fn truncate_per_schedule(runs: &mut Vec<ScheduleRun>, max_per_schedule: usize) -> Result<(), String> {
    // Walk newest-first, keep the first `max_per_schedule` per
    // schedule, mark the rest for removal by id.
    use std::collections::HashSet;
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut keep: HashSet<String> = HashSet::new();
    let mut sorted: Vec<&ScheduleRun> = runs.iter().collect();
    sorted.sort_by_key(|run| std::cmp::Reverse(run.fired_at_ms));
    for run in sorted {
        let count = counts.entry(run.schedule_id.clone()).or_insert(0);
        if *count < max_per_schedule {
            keep.insert(run.id.clone());
            *count += 1;
        }
    }
    runs.retain(|run| keep.contains(&run.id));
    Ok(())
}

/// MCP protocol wrappers. They are thin: they call the matching
/// `Service` method, log the call, and return a JSON summary.
/// `self.require_collaboration_caller` is enforced inside the
/// underlying methods.
impl Service {
    pub(crate) fn list_schedules_protocol(
        self: &Arc<Self>,
        caller_task: &str,
    ) -> Result<serde_json::Value, String> {
        let snap = self.list_schedules()?;
        Ok(serde_json::json!({
            "ok": true,
            "schedules": snap.schedules,
            "runs": snap.schedule_runs,
            "caller": caller_task,
        }))
    }

    pub(crate) fn save_schedule_protocol(
        self: &Arc<Self>,
        caller_task: &str,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let input = serde_json::from_value::<crate::scheduler::ScheduleInput>(
            serde_json::Value::Object(args.clone()),
        )
        .map_err(|e| format!("Invalid schedule input: {e}"))?;
        let snap = self.save_schedule(input)?;
        Ok(serde_json::json!({
            "ok": true,
            "caller": caller_task,
            "schedule_count": snap.schedules.len(),
        }))
    }

    pub(crate) fn delete_schedule_protocol(
        self: &Arc<Self>,
        caller_task: &str,
        id: &str,
    ) -> Result<serde_json::Value, String> {
        let snap = self.delete_schedule(id.to_string())?;
        Ok(serde_json::json!({
            "ok": true,
            "caller": caller_task,
            "schedule_count": snap.schedules.len(),
        }))
    }

    pub(crate) fn run_schedule_now_protocol(
        self: &Arc<Self>,
        caller_task: &str,
        id: &str,
    ) -> Result<serde_json::Value, String> {
        let snap = self.run_schedule_now(id.to_string())?;
        Ok(serde_json::json!({
            "ok": true,
            "caller": caller_task,
            "run_count": snap.schedule_runs.len(),
        }))
    }

    pub(crate) fn pause_schedule_protocol(
        self: &Arc<Self>,
        caller_task: &str,
        id: &str,
    ) -> Result<serde_json::Value, String> {
        let snap = self.set_schedule_enabled(id.to_string(), false)?;
        Ok(serde_json::json!({"ok": true, "caller": caller_task, "schedule_count": snap.schedules.len()}))
    }

    pub(crate) fn resume_schedule_protocol(
        self: &Arc<Self>,
        caller_task: &str,
        id: &str,
    ) -> Result<serde_json::Value, String> {
        let snap = self.set_schedule_enabled(id.to_string(), true)?;
        Ok(serde_json::json!({"ok": true, "caller": caller_task, "schedule_count": snap.schedules.len()}))
    }
}

/// Cleanup pass for `throwaway` schedule fires. Runs from the
/// scheduler tick and at shutdown. For each task id in
/// `pending_throwaway_task_ids`:
/// - status `"running"` — leave it for the next tick.
/// - status `"completed"` — write a Markdown report under the task
///   folder, archive the task, then delete it.
/// - any other status — archive the task so the user can inspect the
///   failure later, and clear it from the pending list.
///
/// Returns the number of throwaway tasks cleaned up this pass.
pub(crate) fn cleanup_throwaway_tasks(service: &Arc<Service>) -> Result<usize, String> {
    let pending: Vec<String> = {
        let data = service
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.".to_string())?;
        data.snapshot.pending_throwaway_task_ids.clone()
    };
    if pending.is_empty() {
        return Ok(0);
    }
    let mut cleaned = 0;
    let mut remaining: Vec<String> = Vec::new();
    for task_id in pending {
        let task_status = match service
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.".to_string())
            .and_then(|data| {
                Ok(data
                    .snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == task_id)
                    .map(|t| (t.status.clone(), t.cwd.clone(), t.title.clone())))
            }) {
            Ok(Some((status, cwd, title))) => (status, cwd, title),
            Ok(None) => continue, // task already gone
            Err(_) => {
                remaining.push(task_id);
                continue;
            }
        };
        let (status, cwd, title) = task_status;
        if status == "running" {
            remaining.push(task_id);
            continue;
        }
        let success = status == "completed";
        if success {
            if let Err(e) = write_throwaway_report(service, &task_id, &cwd, &title, status.clone()) {
                eprintln!("[monitter] throwaway report write failed: {e}");
            }
            // Archive then delete. delete_task_blocking requires
            // the task to be archived first.
            if let Err(e) = service.set_task_archived(&task_id, true) {
                eprintln!("[monitter] throwaway archive failed: {e}");
                remaining.push(task_id);
                continue;
            }
            if let Err(e) = delete_task_blocking_public(service, &task_id) {
                eprintln!("[monitter] throwaway delete failed: {e}");
                remaining.push(task_id);
                continue;
            }
        } else {
            // Failure: archive so the user can inspect.
            if let Err(e) = service.set_task_archived(&task_id, true) {
                eprintln!("[monitter] throwaway archive failed: {e}");
                remaining.push(task_id);
                continue;
            }
        }
        cleaned += 1;
    }
    // Persist the updated pending list.
    service
        .mutate(None, |snap| {
            snap.pending_throwaway_task_ids = remaining.clone();
            Ok(snap.clone())
        })?;
    Ok(cleaned)
}

fn write_throwaway_report(
    service: &Arc<Service>,
    task_id: &str,
    cwd: &str,
    title: &str,
    status: String,
) -> Result<(), String> {
    let snap = service.snapshot()?;
    let task = snap
        .tasks
        .iter()
        .find(|t| t.id == task_id);
    let messages: Vec<(String, String, i64)> = snap
        .messages
        .iter()
        .filter(|m| m.task_id == task_id)
        .map(|m| (m.role.clone(), m.text.clone(), m.created_at))
        .collect();
    let agent = task.and_then(|t| {
        snap.agents
            .iter()
            .find(|a| a.id == t.agent_id)
            .map(|a| a.name.clone())
    });
    let mut report = String::new();
    report.push_str(&format!("# Scheduled throwaway run: {}\n\n", title));
    report.push_str(&format!(
        "- **Task ID**: `{}`\n- **Status**: `{}`\n- **Finished**: `{}`\n- **Agent**: `{}`\n\n",
        task_id,
        status,
        crate::model::now(),
        agent.unwrap_or_else(|| "unknown".into())
    ));
    report.push_str("## Transcript\n\n");
    if messages.is_empty() {
        report.push_str("_(no messages)_\n");
    } else {
        for (role, text, _ts) in messages {
            report.push_str(&format!("**{}**: {}\n\n", role, text));
        }
    }
    let reports_dir = std::path::PathBuf::from(cwd).join(".monitter").join("throwaway-reports");
    std::fs::create_dir_all(&reports_dir)
        .map_err(|e| format!("Cannot create throwaway reports directory: {e}"))?;
    let filename = format!("{}.md", task_id);
    let path = reports_dir.join(&filename);
    std::fs::write(&path, report)
        .map_err(|e| format!("Cannot write throwaway report {}: {e}", path.display()))?;
    Ok(())
}

/// Public re-export of the internal delete path. Kept here so the
/// scheduler cleanup pass can call it without going through a Tauri
/// command. The contract is: throws an error if the task is not
/// archived, is running, or has pending collaborations.
pub(crate) fn delete_task_blocking_public(service: &Arc<Service>, id: &str) -> Result<(), String> {
    service.mutate_data(Some(id.to_string()), |data| {
        if data.snapshot.collaborations.iter().any(|delivery| {
            (delivery.from_task_id == id || delivery.to_task_id == id)
                && matches!(delivery.status.as_str(), "queued" | "running")
        }) {
            return Err("Finish or cancel this chat's pending collaborations before deleting it.".into());
        }
        let task = data
            .snapshot
            .tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| "Task was not found.".to_string())?;
        if !task.archived {
            return Err("Archive this chat before permanently deleting it.".into());
        }
        if task.status == "running" {
            return Err("Cancel a running task before deleting it.".into());
        }
        data.snapshot.tasks.retain(|task| task.id != id);
        data.snapshot
            .messages
            .retain(|message| message.task_id != id);
        data.snapshot.events.retain(|event| event.task_id != id);
        data.snapshot
            .subagent_sessions
            .retain(|session| session.parent_task_id != id);
        service
            .store
            .remove_task_usage(id)
            .map_err(|e| format!("Usage ledger cleanup failed: {e}"))?;
        data.snapshot
            .queued_messages
            .retain(|message| message.task_id != id);
        data.task_hosts.remove(id);
        Ok(())
    })?;
    Ok(())
}
