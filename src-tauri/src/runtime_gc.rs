//! Idle process lifetime is independent from durable chat/turn state.
//!
//! The collector never submits a prompt. Normal send paths wait for a retiring
//! owner, then bootstrap the saved provider session through the usual adapter.

use crate::{runner::RunControl, Service, ServiceData};
use std::{
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, Instant},
};

pub(crate) const IDLE_RUNTIME_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const COLLECT_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const RUNTIME_RELEASE_TIMEOUT: Duration = Duration::from_secs(15);

fn task_allows_retirement(data: &ServiceData, task_id: &str) -> bool {
    let Some(task) = data.snapshot.tasks.iter().find(|task| task.id == task_id) else {
        return false;
    };
    task.status != "running"
        && data
            .task_hosts
            .get(task_id)
            .or_else(|| {
                data.snapshot
                    .hosts
                    .iter()
                    .find(|host| host.id == task.host_id)
            })
            .is_some_and(|host| host.kind == "local")
        && task
            .native_session_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
        && !data.snapshot.queued_messages.iter().any(|message| {
            message.task_id == task_id && matches!(message.status.as_str(), "queued" | "sending")
        })
        && !data
            .snapshot
            .approval_requests
            .iter()
            .any(|request| request.task_id == task_id && request.status == "pending")
}

impl Service {
    pub(crate) fn start_idle_collector(self: &Arc<Self>) {
        if self.idle_collector_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        thread::spawn(move || loop {
            thread::sleep(COLLECT_INTERVAL);
            let Some(service) = weak.upgrade() else { break };
            if service.stopping.load(Ordering::Acquire) {
                break;
            }
            service.collect_idle_runtimes_at(Instant::now());
        });
    }

    /// Called only after completion has been saved. Recheck under data -> runs
    /// so a new accepted send cannot have its clock reset by an old completion.
    pub(crate) fn mark_runtime_idle_if_current(&self, task_id: &str, control: &Arc<RunControl>) {
        let Ok(data) = self.data.lock() else { return };
        if !data
            .snapshot
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.status != "running")
        {
            return;
        }
        let Ok(runs) = self.runs.lock() else { return };
        if runs
            .tasks
            .get(task_id)
            .is_some_and(|current| Arc::ptr_eq(current, control))
        {
            control.mark_idle();
        }
    }

    /// A synchronous sweep makes clock boundaries and send/retire races
    /// testable. Production calls it from one weakly-owned background thread.
    pub(crate) fn collect_idle_runtimes_at(self: &Arc<Self>, now: Instant) -> usize {
        let Ok(_sweep) = self.idle_collection.try_lock() else {
            return 0;
        };
        if self.stopping.load(Ordering::Acquire) {
            return 0;
        }
        let candidates = {
            let Ok(data) = self.data.lock() else { return 0 };
            let Ok(runs) = self.runs.lock() else { return 0 };
            runs.tasks
                .iter()
                .filter(|(task_id, control)| {
                    control.is_retiring()
                        || (control.idle_timeout_elapsed(now, IDLE_RUNTIME_TIMEOUT)
                            && task_allows_retirement(&data, task_id))
                })
                .map(|(task_id, control)| (task_id.clone(), Arc::clone(control)))
                .collect::<Vec<_>>()
        };
        let mut retired = 0;
        for (task_id, control) in candidates {
            // Provider/MCP infrastructure can start after the completed-turn
            // refresh (and even after the collector's previous sweep). While
            // this runtime has never reported tool work, adopt the current
            // owned tree before deciding whether it is safe to retire. The
            // permanent tool-work fence makes this a no-op for any runtime
            // that has crossed a user-tool boundary, where late descendants
            // must continue to pin retirement.
            let _ = control.refresh_runtime_process_baseline_while_idle_if_no_tool_work();
            // Process inspection and all teardown I/O happen outside service
            // locks. Unknown descendants conservatively pin the runtime.
            if !control.is_retiring() && !control.retirement_process_tree_is_safe() {
                continue;
            }
            let claimed = (|| {
                let data = self.data.lock().ok()?;
                let runs = self.runs.lock().ok()?;
                if self.stopping.load(Ordering::Acquire)
                    || self.admin_turn_broker.active_task_id().as_deref() == Some(task_id.as_str())
                    || !runs
                        .tasks
                        .get(&task_id)
                        .is_some_and(|current| Arc::ptr_eq(current, &control))
                {
                    return None;
                }
                if control.is_retiring() {
                    // Retry only cleanup of the same fenced owner, never a
                    // native turn. An accepted send may be waiting for it.
                    Some(())
                } else {
                    (task_allows_retirement(&data, &task_id)
                        && control.try_retire_idle(now, IDLE_RUNTIME_TIMEOUT))
                    .then_some(())
                }
            })()
            .is_some();
            if !claimed {
                continue;
            }
            // From this point new sends can be accepted durably, but their
            // dispatcher must wait for this exact owner's teardown barrier.
            match control.retire_owned() {
                Ok(()) => {
                    self.release_retired_run(&task_id, &control);
                    retired += 1;
                }
                Err(error) => {
                    if !control.is_retiring() {
                        // A new unknown child appeared before shutdown began.
                        // The runtime returned to idle and remains usable.
                        continue;
                    }
                    // Keep the native writer fenced. A subsequent send gets a
                    // bounded failure rather than launching a second writer.
                    let first_failure = self
                        .idle_retirement_failures
                        .lock()
                        .map(|mut failures| {
                            failures.insert((task_id.clone(), Arc::as_ptr(&control) as usize))
                        })
                        .unwrap_or(false);
                    if first_failure && !self.is_internal_admin_task(&task_id) {
                        self.record(
                            &task_id,
                            "error",
                            "Idle runtime could not be retired",
                            error,
                        );
                    }
                }
            }
        }
        retired
    }

    pub(crate) fn release_retired_run(&self, task_id: &str, control: &Arc<RunControl>) {
        self.release_app_server_run(task_id, control);
        if let Ok(mut failures) = self.idle_retirement_failures.lock() {
            failures.remove(&(task_id.into(), Arc::as_ptr(control) as usize));
        }
        control.finish_retirement();
    }

    /// Never mistake a retiring/cancelled registry entry for an absent one.
    /// Holding the writer claim until reap prevents overlapping cold resumes.
    pub(crate) fn available_runtime(
        &self,
        task_id: &str,
    ) -> Result<Option<Arc<RunControl>>, String> {
        let deadline = Instant::now() + RUNTIME_RELEASE_TIMEOUT;
        loop {
            if self.stopping.load(Ordering::Acquire) {
                return Err("Monitter is shutting down.".into());
            }
            let control = self
                .runs
                .lock()
                .map_err(|_| "Monitter run registry lock failed.".to_string())?
                .tasks
                .get(task_id)
                .cloned();
            let Some(control) = control else {
                return Ok(None);
            };
            if control.is_retiring() {
                control.wait_for_teardown(deadline.saturating_duration_since(Instant::now()))?;
                continue;
            }
            if control.is_cancelled() {
                if Instant::now() >= deadline {
                    return Err("The previous agent process has not finished stopping. Its saved session is preserved.".into());
                }
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            return Ok(Some(control));
        }
    }
}
