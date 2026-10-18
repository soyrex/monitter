//! Read-only reconciliation of recorded Codex children on their existing owner.
//! Child facts have a separate authority fence from parent-turn transcript writes.
use crate::{
    Service, ServiceData,
    model::{Snapshot, SubagentSession, now},
    runner::RunControl,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

const MAX_PROBES: usize = 4;
const CHILD_INTERVAL: Duration = Duration::from_secs(60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_LINEAGE: usize = 16;
const MAX_DETAIL: usize = 16 * 1024;

#[derive(Default)]
pub(crate) struct ReconciliationState {
    owners: HashMap<(String, usize), OwnerProbe>,
    children: HashMap<(String, String, usize), ChildProbe>,
}
#[derive(Default)]
struct ChildProbe {
    last: Option<Instant>,
    last_wake: Option<Instant>,
    wake_pending: bool,
}
#[derive(Default)]
struct OwnerProbe {
    control: Option<Weak<RunControl>>,
    last: Option<Instant>,
    next: Option<Instant>,
    failures: u32,
    suppressed: bool,
    waiting_for_query: bool,
    version: u64,
}

#[derive(Clone)]
struct Probe {
    task_id: String,
    root: String,
    control: Arc<RunControl>,
    run_id: String,
    turn_id: Option<String>,
    session: SubagentSession,
    version: u64,
}
impl Probe {
    fn owner_key(&self) -> (String, usize) {
        (self.task_id.clone(), Arc::as_ptr(&self.control) as usize)
    }
    fn child_key(&self) -> (String, String, usize) {
        (
            self.task_id.clone(),
            self.session.id.clone(),
            Arc::as_ptr(&self.control) as usize,
        )
    }
}

fn terminal(status: &str) -> bool {
    matches!(status, "completed" | "error" | "interrupted")
}
fn bounded_detail(value: &str) -> String {
    value.chars().take(MAX_DETAIL).collect()
}
fn known_child<'a>(
    snapshot: &'a Snapshot,
    task: &str,
    thread: &str,
) -> Option<&'a SubagentSession> {
    let mut rows = snapshot.subagent_sessions.iter().filter(|s| {
        s.source == "codex"
            && s.parent_task_id == task
            && s.id == format!("codex:{thread}")
            && s.agent_thread_id.as_deref() == Some(thread)
    });
    let row = rows.next()?;
    if rows.next().is_some() {
        None
    } else {
        Some(row)
    }
}

// A missing stored parent is common for subAgentActivity. Its task-scoped,
// recorded membership remains an ownership anchor; do not invent a parent ID.
// When a parent is present, every recorded link must remain in this owner.
fn lineage(
    snapshot: &Snapshot,
    task: &str,
    root: &str,
    session: &SubagentSession,
    source_parent: Option<&str>,
) -> bool {
    if session.agent_thread_id.as_deref() == Some(root) {
        return false;
    }
    if let (Some(stored), Some(source)) = (session.parent_thread_id.as_deref(), source_parent) {
        if stored != source {
            return false;
        }
    }
    let mut parent = source_parent.or(session.parent_thread_id.as_deref());
    let mut seen = HashSet::from([session.agent_thread_id.as_deref().unwrap_or("")]);
    for _ in 0..MAX_LINEAGE {
        let Some(thread) = parent else {
            return source_parent.is_none();
        };
        if thread == root {
            return true;
        }
        if !seen.insert(thread) {
            return false;
        }
        let Some(recorded) = known_child(snapshot, task, thread) else {
            return false;
        };
        parent = recorded.parent_thread_id.as_deref();
    }
    false
}

fn owner_current(data: &ServiceData, control: &RunControl, task: &str, root: &str) -> bool {
    !control.is_cancelled()
        && !control.is_planned_retirement()
        && control.current_app_server_thread().as_deref() == Some(root)
        && data.snapshot.tasks.iter().any(|t| {
            t.id == task && t.provider == "codex" && t.native_session_id.as_deref() == Some(root)
        })
}
// Child reads are independent from a parent turn. During execution, require
// the established native turn; a pending send/accepted receipt is transitional.
fn owner_ready_for_probe(data: &ServiceData, control: &RunControl, task: &str) -> bool {
    control.is_resident()
        && !control.has_pending_native_turn()
        && !control.has_app_server_native_turn_request()
        && !control.has_app_server_turn_request()
        && data.snapshot.tasks.iter().any(|t| {
            t.id == task
                && !t.archived
                && (!(control.is_active() || t.status == "running")
                    || control
                        .current_app_server_turn()
                        .is_some_and(|id| !id.trim().is_empty()))
        })
        && !data.accepted_turns.contains_key(task)
}

#[derive(Debug, PartialEq)]
struct TerminalObservation {
    status: &'static str,
    error: Option<String>,
    result: Option<String>,
    completed_at: Option<i64>,
}
fn completion_time(turn: &Value, created_at: i64) -> Option<i64> {
    let at = turn.get("completedAt")?.as_i64()?.checked_mul(1000)?;
    (at > 0 && at >= created_at && at <= now().saturating_add(60_000)).then_some(at)
}
fn turn_terminal(turn: &Value) -> Option<TerminalObservation> {
    let status = match turn.get("status")?.as_str()? {
        "completed" => "completed",
        "failed" => "error",
        "interrupted" => "interrupted",
        _ => return None,
    };
    let error = turn
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(bounded_detail);
    let result = turn
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .rev()
                .find(|i| i.get("type").and_then(Value::as_str) == Some("agentMessage"))
        })
        .and_then(|i| i.get("text"))
        .and_then(Value::as_str)
        .map(bounded_detail);
    Some(TerminalObservation {
        status,
        error,
        result,
        completed_at: None,
    })
}

fn read_terminal(snapshot: &Snapshot, probe: &Probe, value: &Value) -> Option<TerminalObservation> {
    let thread = value.get("thread")?;
    if thread.get("id")?.as_str()? != probe.session.agent_thread_id.as_deref()? {
        return None;
    }
    match thread.pointer("/status/type")?.as_str()? {
        "idle" | "notLoaded" | "systemError" => {}
        _ => return None,
    }
    let source = thread
        .pointer("/source/subAgent/thread_spawn/parent_thread_id")?
        .as_str()?;
    if !lineage(
        snapshot,
        &probe.task_id,
        &probe.root,
        &probe.session,
        Some(source),
    ) {
        return None;
    }
    let turns = thread.get("turns")?.as_array()?;
    if turns.is_empty() || turns.len() > 512 {
        return None;
    }
    // Refuse any live turn even when another, older turn is terminal.
    if turns.iter().any(|t| {
        !matches!(
            t.get("status").and_then(Value::as_str),
            Some("completed" | "failed" | "interrupted")
        )
    }) {
        return None;
    }
    let latest = if turns.len() == 1 {
        &turns[0]
    } else {
        // Missing/tied timestamps do not establish which turn is latest.
        let mut ordered = turns
            .iter()
            .map(|t| Some((t.get("startedAt")?.as_i64()?, t)))
            .collect::<Option<Vec<_>>>()?;
        ordered.sort_by_key(|(started, _)| *started);
        let (last, turn) = ordered.last()?;
        if ordered
            .get(ordered.len() - 2)
            .is_some_and(|(previous, _)| previous == last)
        {
            return None;
        }
        *turn
    };
    let mut observation = turn_terminal(latest)?;
    observation.completed_at = completion_time(latest, probe.session.created_at);
    Some(observation)
}

impl ReconciliationState {
    fn select(
        &mut self,
        mut candidates: Vec<Probe>,
        now: Instant,
        live_owners: &HashMap<String, Arc<RunControl>>,
    ) -> Vec<Probe> {
        self.owners.retain(|(task, pointer), state| {
            live_owners.get(task).is_some_and(|current| {
                Arc::as_ptr(current) as usize == *pointer
                    && state
                        .control
                        .as_ref()
                        .and_then(Weak::upgrade)
                        .is_some_and(|old| Arc::ptr_eq(&old, current))
            })
        });
        self.children
            .retain(|(task, _, owner), _| self.owners.contains_key(&(task.clone(), *owner)));
        candidates.retain(|p| {
            let owner = self.owners.get(&p.owner_key());
            !owner.is_some_and(|o| {
                o.suppressed
                    || (o.waiting_for_query
                        && p.control.pending_app_server_query_count().unwrap_or(1) > 0)
                    || o.next.is_some_and(|next| now < next)
            }) && !self.children.get(&p.child_key()).is_some_and(|child| {
                !child.wake_pending
                    && child
                        .last
                        .is_some_and(|last| now.saturating_duration_since(last) < CHILD_INTERVAL)
            })
        });
        candidates.sort_by_key(|p| {
            (
                self.owners.get(&p.owner_key()).and_then(|o| o.last),
                !self
                    .children
                    .get(&p.child_key())
                    .is_some_and(|child| child.wake_pending),
                self.children
                    .get(&p.child_key())
                    .and_then(|child| child.last),
                p.task_id.clone(),
                p.session.id.clone(),
            )
        });
        let mut selected = Vec::new();
        let mut picked = HashSet::new();
        for mut probe in candidates {
            if !picked.insert(probe.owner_key()) {
                continue;
            }
            let owner = self.owners.entry(probe.owner_key()).or_default();
            owner.control = Some(Arc::downgrade(&probe.control));
            owner.last = Some(now);
            probe.version = owner.version;
            let child = self.children.entry(probe.child_key()).or_default();
            child.last = Some(now);
            if child.wake_pending {
                child.last_wake = Some(now);
                child.wake_pending = false;
            }
            selected.push(probe);
            if selected.len() == MAX_PROBES {
                break;
            }
        }
        selected
    }
    fn outcome(&mut self, probe: &Probe, now: Instant, outcome: &Result<Value, String>) {
        let owner = self.owners.entry(probe.owner_key()).or_default();
        match outcome {
            Ok(_) => {
                owner.failures = 0;
                owner.next = None;
                owner.waiting_for_query = false;
            }
            Err(error) => {
                owner.failures = owner.failures.saturating_add(1);
                let error = error.to_ascii_lowercase();
                // Timed-out RPC IDs remain retained by RunControl. Wait until
                // the reader consumes them, then retry after bounded backoff.
                owner.waiting_for_query =
                    error.contains("timed out") || error.contains("too many unanswered");
                owner.suppressed = error.contains("method not found")
                    || error.contains("not supported")
                    || error.contains("unknown method")
                    || error.contains("invalid params");
                owner.next = Some(
                    now + Duration::from_secs(
                        (60u64.saturating_mul(1u64 << owner.failures.min(4))).min(600),
                    ),
                );
            }
        }
    }
}

impl Service {
    fn subagent_probes(&self) -> Vec<Probe> {
        let Ok(data) = self.data.lock() else {
            return vec![];
        };
        let Ok(runs) = self.runs.lock() else {
            return vec![];
        };
        data.snapshot
            .subagent_sessions
            .iter()
            .filter(|s| s.source == "codex" && !terminal(&s.status))
            .filter_map(|session| {
                let control = runs.tasks.get(&session.parent_task_id)?.clone();
                let root = control.current_app_server_thread()?;
                if !owner_current(&data, &control, &session.parent_task_id, &root)
                    || !owner_ready_for_probe(&data, &control, &session.parent_task_id)
                {
                    return None;
                }
                let child = session.agent_thread_id.as_deref()?;
                known_child(&data.snapshot, &session.parent_task_id, child)?;
                if !lineage(
                    &data.snapshot,
                    &session.parent_task_id,
                    &root,
                    session,
                    None,
                ) {
                    return None;
                }
                Some(Probe {
                    task_id: session.parent_task_id.clone(),
                    root,
                    run_id: control.current_run_id().ok()?,
                    turn_id: control.current_app_server_turn(),
                    control,
                    session: session.clone(),
                    version: 0,
                })
            })
            .collect()
    }
    fn subagent_probe_is_current(&self, probe: &Probe) -> bool {
        let Ok(data) = self.data.lock() else {
            return false;
        };
        let Ok(runs) = self.runs.lock() else {
            return false;
        };
        if !owner_current(&data, &probe.control, &probe.task_id, &probe.root)
            || !owner_ready_for_probe(&data, &probe.control, &probe.task_id)
            || !runs
                .tasks
                .get(&probe.task_id)
                .is_some_and(|owner| Arc::ptr_eq(owner, &probe.control))
            || probe.control.current_run_id().ok().as_deref() != Some(probe.run_id.as_str())
            || probe.control.current_app_server_turn() != probe.turn_id
        {
            return false;
        }
        let Some(child) = probe.session.agent_thread_id.as_deref() else {
            return false;
        };
        if known_child(&data.snapshot, &probe.task_id, child) != Some(&probe.session) {
            return false;
        }
        self.subagent_reconciliation
            .lock()
            .ok()
            .is_some_and(|state| {
                !state
                    .owners
                    .get(&probe.owner_key())
                    .is_some_and(|owner| owner.version != probe.version)
            })
    }

    fn apply_subagent_read(&self, probe: &Probe, value: &Value) -> Result<bool, String> {
        self.mutate_data(Some(probe.task_id.clone()), |data| {
            let runs = self.runs.lock().map_err(|_| "Run registry unavailable.")?;
            if !owner_current(data, &probe.control, &probe.task_id, &probe.root)
                || !owner_ready_for_probe(data, &probe.control, &probe.task_id)
                || !runs
                    .tasks
                    .get(&probe.task_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &probe.control))
                || probe.control.current_run_id()? != probe.run_id
                || probe.control.current_app_server_turn() != probe.turn_id
            {
                return Ok(false);
            }
            if self
                .subagent_reconciliation
                .lock()
                .map_err(|_| "Child reconciliation state unavailable.")?
                .owners
                .get(&probe.owner_key())
                .is_some_and(|owner| owner.version != probe.version)
            {
                return Ok(false);
            }
            let child = probe
                .session
                .agent_thread_id
                .as_deref()
                .ok_or("Missing child thread.")?;
            let Some(current) = known_child(&data.snapshot, &probe.task_id, child) else {
                return Ok(false);
            };
            if current != &probe.session || terminal(&current.status) {
                return Ok(false);
            }
            let Some(observation) = read_terminal(&data.snapshot, probe, value) else {
                return Ok(false);
            };
            let row = data
                .snapshot
                .subagent_sessions
                .iter_mut()
                .find(|s| s.id == probe.session.id)
                .ok_or("Child disappeared.")?;
            row.status = observation.status.into();
            row.error = observation.error;
            if observation.result.is_some() {
                row.result = observation.result;
            }
            row.updated_at = observation.completed_at.unwrap_or_else(now);
            Ok(true)
        })
    }
    // Reader notifications only invalidate a probe, never infer completion from
    // an old child turn or a parent's envelope. The subsequent read establishes
    // the current child turn without blocking the stdout reader on itself.
    pub(crate) fn observe_codex_child_notification(
        &self,
        task_id: &str,
        control: &Arc<RunControl>,
        value: &Value,
    ) {
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        if !matches!(
            method,
            "turn/started"
                | "turn/completed"
                | "thread/status/changed"
                | "item/started"
                | "item/completed"
        ) {
            return;
        }
        let params = &value["params"];
        let Some(reporter) = params
            .get("threadId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        let Some(root) = control.current_app_server_thread() else {
            return;
        };
        let Ok(_writer) = self.state_writes.lock() else {
            return;
        };
        let Ok(data) = self.data.lock() else {
            return;
        };
        let Ok(runs) = self.runs.lock() else {
            return;
        };
        if !owner_current(&data, control, task_id, &root)
            || !runs
                .tasks
                .get(task_id)
                .is_some_and(|current| Arc::ptr_eq(current, control))
        {
            return;
        }
        if reporter != root
            && !known_child(&data.snapshot, task_id, reporter)
                .is_some_and(|s| lineage(&data.snapshot, task_id, &root, s, None))
        {
            return;
        }
        let targets = if matches!(method, "item/started" | "item/completed") {
            crate::runner::parse_codex_subagent_updates(&params["item"], task_id)
                .into_iter()
                .filter_map(|u| {
                    u.agent_thread_id
                        .map(|thread| (thread, u.status.as_deref().is_some_and(terminal)))
                })
                .collect::<Vec<_>>()
        } else {
            let finished = method == "turn/completed"
                || (method == "thread/status/changed"
                    && matches!(
                        params.pointer("/status/type").and_then(Value::as_str),
                        Some("idle" | "notLoaded" | "systemError")
                    ));
            vec![(reporter.to_owned(), finished)]
        };
        let known_targets = targets
            .into_iter()
            .filter(|(target, _)| {
                known_child(&data.snapshot, task_id, target).is_some_and(|s| {
                    !terminal(&s.status) && lineage(&data.snapshot, task_id, &root, s, None)
                })
            })
            .collect::<Vec<_>>();
        if known_targets.is_empty() {
            return;
        }
        if let Ok(mut state) = self.subagent_reconciliation.lock() {
            let pointer = Arc::as_ptr(control) as usize;
            let owner = state.owners.entry((task_id.into(), pointer)).or_default();
            owner.control = Some(Arc::downgrade(control));
            owner.version = owner.version.wrapping_add(1);
            let at = Instant::now();
            for (target, finished) in known_targets {
                if !finished {
                    continue;
                }
                // One expedited read per child per interval. Duplicate/old
                // completion hints cannot defeat polling or error backoff.
                let child = state
                    .children
                    .entry((task_id.into(), format!("codex:{target}"), pointer))
                    .or_default();
                if !child
                    .last_wake
                    .is_some_and(|last| at.saturating_duration_since(last) < CHILD_INTERVAL)
                {
                    child.wake_pending = true;
                }
            }
        }
    }

    fn reconcile_codex_subagents_with(
        &self,
        at: Instant,
        mut query: impl FnMut(&Probe) -> Result<Value, String>,
    ) -> usize {
        let candidates = self.subagent_probes();
        let live_owners = match self.runs.lock() {
            Ok(runs) => runs.tasks.clone(),
            Err(_) => return 0,
        };
        let probes = match self.subagent_reconciliation.lock() {
            Ok(mut state) => state.select(candidates, at, &live_owners),
            Err(_) => return 0,
        };
        let mut changed = 0;
        for probe in probes {
            // Earlier owners may have spent the timeout on I/O while this
            // owner accepted a fresh turn. Recheck before issuing each read.
            if !self.subagent_probe_is_current(&probe) {
                continue;
            }
            // No service, registry or reconciliation lock is held during RPC.
            let response = query(&probe);
            if let Ok(mut state) = self.subagent_reconciliation.lock() {
                state.outcome(&probe, at, &response);
            }
            if let Ok(value) = response {
                if self.apply_subagent_read(&probe, &value).unwrap_or(false) {
                    changed += 1;
                }
            }
        }
        changed
    }
    pub(crate) fn reconcile_codex_subagents_at(&self, at: Instant) -> usize {
        self.reconcile_codex_subagents_with(at, |probe| {
            probe.control.query_app_server(
                "thread/read",
                json!({"threadId":probe.session.agent_thread_id,"includeTurns":true}),
                QUERY_TIMEOUT,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CreateTaskInput, SubagentSessionUpdate, upsert_subagent_session};
    struct Fixture {
        service: Arc<Service>,
        directory: std::path::PathBuf,
        task: String,
        owner: Arc<RunControl>,
    }
    impl Fixture {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir()
                .join(format!("monitter-child-reconcile-{name}-{}", crate::id()));
            let service = Service::open(None, directory.clone()).unwrap();
            let agent = service.snapshot().unwrap().agents[0].clone();
            let task = service
                .create_task(CreateTaskInput {
                    agent_id: agent.id,
                    title: "Recorded parent".into(),
                    native_session_id: Some("native-root".into()),
                    parent_task_id: None,
                    channel_id: None,
                    project_id: None,
                    cwd: None,
                    model_settings: None,
                    sandbox: None,
                })
                .unwrap();
            let owner = RunControl::new(false);
            owner.begin_run().unwrap();
            owner.mark_resident();
            owner.set_app_server_thread("native-root".into());
            owner.mark_idle();
            service
                .runs
                .lock()
                .unwrap()
                .tasks
                .insert(task.id.clone(), owner.clone());
            service
                .mutate(None, |s| {
                    s.tasks.iter_mut().find(|t| t.id == task.id).unwrap().status =
                        "completed".into();
                    s.subagent_sessions.push(SubagentSession {
                        id: "codex:child".into(),
                        source: "codex".into(),
                        parent_task_id: task.id.clone(),
                        parent_thread_id: None,
                        collaboration_id: None,
                        agent_path: Some("/root/codex_steering".into()),
                        agent_thread_id: Some("child".into()),
                        prompt: None,
                        model: None,
                        reasoning_effort: None,
                        status: "running".into(),
                        result: None,
                        error: None,
                        created_at: now() - 20_000,
                        updated_at: now(),
                    });
                    Ok(())
                })
                .unwrap();
            Self {
                service,
                directory,
                task: task.id,
                owner,
            }
        }
        fn start_parent(&self) {
            self.owner.begin_run().unwrap();
            self.owner
                .set_app_server_turn("established-parent-turn".into());
            self.service
                .mutate(None, |s| {
                    s.tasks
                        .iter_mut()
                        .find(|t| t.id == self.task)
                        .unwrap()
                        .status = "running".into();
                    Ok(())
                })
                .unwrap();
        }
        fn probe(&self) -> Probe {
            self.service.subagent_probes().pop().unwrap()
        }
        fn child(&self) -> SubagentSession {
            self.service.snapshot().unwrap().subagent_sessions[0].clone()
        }
        fn read(&self) -> Value {
            let completed = now() / 1000 - 3;
            json!({"thread":{"id":"child","status":{"type":"notLoaded"},"source":{"subAgent":{"thread_spawn":{"parent_thread_id":"native-root","depth":1,"agent_path":"/root/codex_steering"}}},"turns":[{"id":"failed-turn","status":"failed","error":{"message":"model unsupported with ChatGPT account","codexErrorInfo":"other"},"startedAt":completed-10,"completedAt":completed}]}})
        }
        fn notify(&self, value: Value) {
            self.service
                .observe_codex_child_notification(&self.task, &self.owner, &value);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.service.cleanup();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn subagent_reconciliation_heals_failed_idle_child_with_native_occurrence_time() {
        let f = Fixture::new("failed");
        let value = f.read();
        let created = f.child().created_at;
        let changed = f
            .service
            .reconcile_codex_subagents_with(Instant::now(), |_| {
                assert!(f.service.data.try_lock().is_ok());
                assert!(f.service.runs.try_lock().is_ok());
                assert!(f.service.state_writes.try_lock().is_ok());
                assert!(f.service.subagent_reconciliation.try_lock().is_ok());
                Ok(value.clone())
            });
        assert_eq!(changed, 1);
        let child = f.child();
        assert_eq!(child.status, "error");
        assert_eq!(
            child.error.as_deref(),
            Some("model unsupported with ChatGPT account")
        );
        assert_eq!(
            child.updated_at,
            value
                .pointer("/thread/turns/0/completedAt")
                .unwrap()
                .as_i64()
                .unwrap()
                * 1000
        );
        assert_eq!(child.created_at, created);
        assert_eq!(
            f.service
                .snapshot()
                .unwrap()
                .tasks
                .iter()
                .find(|t| t.id == f.task)
                .unwrap()
                .status,
            "completed"
        );
        f.service
            .mutate(None, |s| {
                upsert_subagent_session(
                    &mut s.subagent_sessions,
                    SubagentSessionUpdate {
                        id: "codex:child".into(),
                        source: "codex".into(),
                        parent_task_id: f.task.clone(),
                        status: Some("running".into()),
                        agent_path: Some("/root/codex_steering".into()),
                        ..Default::default()
                    },
                    now() + 50_000,
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(f.child().status, "error");
        assert_eq!(f.child().updated_at, child.updated_at);
    }

    #[test]
    fn subagent_reconciliation_heals_terminal_children_during_established_parent_turn() {
        for status in ["completed", "failed"] {
            let f = Fixture::new(status);
            f.start_parent();
            // Neither an approval for the parent nor a queued next message
            // changes the authority of a read of its recorded child.
            f.service
                .mutate(None, |s| {
                    s.approval_requests.push(
                        serde_json::from_value(json!({
                            "id":"approval", "taskId":f.task, "provider":"codex",
                            "runId":f.owner.current_run_id().unwrap(), "tool":"shell",
                            "summary":"Parent approval", "detail":"", "risk":"low",
                            "status":"pending", "createdAt":now()
                        }))
                        .unwrap(),
                    );
                    s.queued_messages.push(serde_json::from_value(json!({
                    "id":"queued", "taskId":f.task, "channelId":null,
                    "text":"Later request", "attachmentIds":[], "createdAt":now(), "status":"queued"
                })).unwrap());
                    Ok(())
                })
                .unwrap();
            let before = f.service.snapshot().unwrap();
            let run = f.owner.current_run_id().unwrap();
            let turn = f.owner.current_app_server_turn();
            let mut value = f.read();
            value["thread"]["turns"][0]["status"] = json!(status);
            assert_eq!(
                f.service
                    .reconcile_codex_subagents_with(Instant::now(), |_| Ok(value.clone())),
                1
            );
            assert_eq!(
                f.child().status,
                if status == "failed" {
                    "error"
                } else {
                    "completed"
                }
            );
            let after = f.service.snapshot().unwrap();
            assert_eq!(before.tasks, after.tasks);
            assert_eq!(before.approval_requests, after.approval_requests);
            assert_eq!(before.queued_messages, after.queued_messages);
            assert_eq!(f.owner.current_run_id().unwrap(), run);
            assert_eq!(f.owner.current_app_server_turn(), turn);
            assert!(f.owner.is_active());
            // Child observation never grants retirement of an active parent.
            let future = Instant::now() + Duration::from_secs(3600);
            assert!(
                !f.owner
                    .try_retire_idle(future, crate::runtime_gc::IDLE_RUNTIME_TIMEOUT)
            );
            assert_eq!(f.service.collect_idle_runtimes_at(future), 0);
            assert!(Arc::ptr_eq(
                f.service.runs.lock().unwrap().tasks.get(&f.task).unwrap(),
                &f.owner
            ));
        }
    }

    #[test]
    fn subagent_reconciliation_active_parent_rechecks_run_and_turn_during_read() {
        for change in ["run", "turn"] {
            let f = Fixture::new(change);
            f.start_parent();
            let value = f.read();
            assert_eq!(
                f.service
                    .reconcile_codex_subagents_with(Instant::now(), |_| {
                        if change == "run" {
                            f.owner.begin_run().unwrap();
                        } else {
                            f.owner.set_app_server_turn("next-parent-turn".into());
                        }
                        Ok(value.clone())
                    }),
                0
            );
            assert_eq!(f.child().status, "running");
            assert!(f.owner.is_active());
        }
    }

    #[test]
    fn subagent_reconciliation_active_parent_preserves_live_child_and_pending_send_fences() {
        let f = Fixture::new("active-child");
        f.start_parent();
        let mut active = f.read();
        active["thread"]["status"] = json!({"type":"active"});
        active["thread"]["turns"][0]["status"] = json!("inProgress");
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(Instant::now(), |_| Ok(active.clone())),
            0
        );
        assert_eq!(f.child().status, "running");
        assert!(f.owner.is_active());
        f.owner
            .set_pending_native_turn(crate::runner::NativeTurnCommand::Compact);
        assert!(f.service.subagent_probes().is_empty());
        f.owner.take_pending_native_turn();
        f.owner.mark_app_server_turn_request(99).unwrap();
        assert!(f.service.subagent_probes().is_empty());
        assert!(f.owner.take_app_server_turn_request(99));
        f.owner.clear_app_server_turn();
        assert!(f.service.subagent_probes().is_empty());
        f.owner
            .set_app_server_turn("established-parent-turn".into());
        f.service
            .mutate_data(None, |data| {
                data.accepted_turns.insert(
                    f.task.clone(),
                    crate::AcceptedTurn {
                        receipt: crate::id(),
                        prompt: "Accepted send".into(),
                        slash_command: None,
                    },
                );
                Ok(())
            })
            .unwrap();
        assert!(f.service.subagent_probes().is_empty());
    }

    #[test]
    fn subagent_completion_notification_wakes_one_bounded_read_during_parent_turn() {
        let f = Fixture::new("completion-wake");
        f.start_parent();
        let at = Instant::now();
        let mut active = f.read();
        active["thread"]["status"] = json!({"type":"active"});
        active["thread"]["turns"][0]["status"] = json!("inProgress");
        let mut calls = 0;
        assert_eq!(
            f.service.reconcile_codex_subagents_with(at, |_| {
                calls += 1;
                Ok(active.clone())
            }),
            0
        );
        f.notify(json!({"method":"turn/completed","params":{"threadId":"child","turn":{"id":"child-turn","status":"completed"}}}));
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + Duration::from_secs(1), |_| {
                    calls += 1;
                    Ok(active.clone())
                }),
            0
        );
        // Repeated/stale completion hints may invalidate reads but do not
        // repeatedly bypass the child interval or prematurely close a live turn.
        for seconds in 2..5 {
            f.notify(json!({"method":"turn/completed","params":{"threadId":"child","turn":{"id":"old-turn","status":"completed"}}}));
            assert_eq!(
                f.service
                    .reconcile_codex_subagents_with(at + Duration::from_secs(seconds), |_| panic!(
                        "duplicate wake must remain bounded"
                    )),
                0
            );
        }
        assert_eq!(calls, 2);
        assert_eq!(f.child().status, "running");
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + Duration::from_secs(61), |_| Ok(f.read())),
            1
        );
        assert_eq!(f.child().status, "error");
        assert!(f.owner.is_active());
    }

    #[test]
    fn subagent_completion_wake_preserves_query_error_backoff() {
        let f = Fixture::new("wake-backoff");
        f.start_parent();
        let at = Instant::now();
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at, |_| Err("query timed out".into())),
            0
        );
        f.notify(json!({"method":"thread/status/changed","params":{"threadId":"child","status":{"type":"idle"}}}));
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + Duration::from_secs(1), |_| panic!(
                    "wake must respect timeout backoff"
                )),
            0
        );
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + Duration::from_secs(121), |_| Ok(f.read())),
            1
        );
    }

    #[test]
    fn subagent_reconciliation_uses_latest_turn_and_refuses_active_or_ambiguous_reads() {
        let f = Fixture::new("latest");
        let probe = f.probe();
        let base = f.read();
        let mut latest = base.clone();
        let started = now() / 1000 - 1;
        latest["thread"]["turns"].as_array_mut().unwrap().push(json!({"id":"latest","status":"completed","startedAt":started,"completedAt":started,"items":[{"type":"agentMessage","text":"Latest result"}]}));
        let observation = read_terminal(&f.service.snapshot().unwrap(), &probe, &latest).unwrap();
        assert_eq!(observation.status, "completed");
        assert_eq!(observation.error, None);
        assert_eq!(observation.result.as_deref(), Some("Latest result"));
        let mut cases = vec![];
        let mut active = latest.clone();
        active["thread"]["turns"][1]["status"] = json!("inProgress");
        cases.push(active);
        let mut approval = base.clone();
        approval["thread"]["status"] = json!({"type":"active","activeFlags":["waitingOnApproval"]});
        cases.push(approval);
        let mut missing = latest.clone();
        missing["thread"]["turns"][1]
            .as_object_mut()
            .unwrap()
            .remove("startedAt");
        cases.push(missing);
        let mut tied = latest.clone();
        tied["thread"]["turns"][1]["startedAt"] = tied["thread"]["turns"][0]["startedAt"].clone();
        cases.push(tied);
        let mut wrong = base.clone();
        wrong["thread"]["id"] = json!("unrelated");
        cases.push(wrong);
        let mut wrong = base.clone();
        wrong["thread"]["source"]["subAgent"]["thread_spawn"]["parent_thread_id"] =
            json!("another-root");
        cases.push(wrong);
        let mut missing = base.clone();
        missing["thread"].as_object_mut().unwrap().remove("status");
        cases.push(missing);
        let mut empty = base.clone();
        empty["thread"]["turns"] = json!([]);
        cases.push(empty);
        for value in cases {
            assert_eq!(
                read_terminal(&f.service.snapshot().unwrap(), &probe, &value),
                None
            );
            assert!(!f.service.apply_subagent_read(&probe, &value).unwrap());
        }
        assert_eq!(f.child().status, "running");
    }

    #[test]
    fn subagent_reconciliation_read_rechecks_owner_run_session_and_pending_turn() {
        for change in ["replace", "cancel", "run", "session", "root", "accepted"] {
            let f = Fixture::new(change);
            let value = f.read();
            let changed = f
                .service
                .reconcile_codex_subagents_with(Instant::now(), |_| {
                    match change {
                        "replace" => {
                            f.service
                                .runs
                                .lock()
                                .unwrap()
                                .tasks
                                .insert(f.task.clone(), RunControl::new(false));
                        }
                        "cancel" => f.owner.cancel(),
                        "run" => {
                            f.owner.begin_run().unwrap();
                            f.owner.mark_idle();
                        }
                        "session" => {
                            f.service
                                .mutate(None, |s| {
                                    s.subagent_sessions[0].updated_at += 1;
                                    Ok(())
                                })
                                .unwrap();
                        }
                        "root" => f.owner.set_app_server_thread("replacement-root".into()),
                        "accepted" => {
                            f.service
                                .mutate_data(None, |d| {
                                    d.accepted_turns.insert(
                                        f.task.clone(),
                                        crate::AcceptedTurn {
                                            receipt: crate::id(),
                                            prompt: "New accepted request".into(),
                                            slash_command: None,
                                        },
                                    );
                                    Ok(())
                                })
                                .unwrap();
                        }
                        _ => unreachable!(),
                    }
                    Ok(value.clone())
                });
            assert_eq!(changed, 0, "{change}");
            assert_eq!(f.child().status, "running", "{change}");
        }
    }

    #[test]
    fn subagent_reconciliation_requires_verified_nested_lineage() {
        let f = Fixture::new("nested");
        let mut snapshot = f.service.snapshot().unwrap();
        let mut parent = snapshot.subagent_sessions[0].clone();
        parent.id = "codex:intermediate".into();
        parent.agent_thread_id = Some("intermediate".into());
        parent.parent_thread_id = Some("native-root".into());
        snapshot.subagent_sessions.push(parent);
        snapshot.subagent_sessions[0].parent_thread_id = Some("intermediate".into());
        let mut probe = f.probe();
        probe.session = snapshot.subagent_sessions[0].clone();
        let mut value = f.read();
        value["thread"]["source"]["subAgent"]["thread_spawn"]["parent_thread_id"] =
            json!("intermediate");
        assert!(read_terminal(&snapshot, &probe, &value).is_some());
        snapshot.subagent_sessions[1].parent_thread_id = None;
        assert!(read_terminal(&snapshot, &probe, &value).is_none());
        snapshot.subagent_sessions[1].parent_thread_id = Some("child".into());
        assert!(read_terminal(&snapshot, &probe, &value).is_none());
        snapshot.subagent_sessions[1].parent_thread_id = Some("other-root".into());
        assert!(read_terminal(&snapshot, &probe, &value).is_none());
    }

    #[test]
    fn subagent_notifications_invalidate_reads_without_completing_old_child_turns() {
        let f = Fixture::new("notification-fence");
        let at = Instant::now();
        let value = f.read();
        let changed=f.service.reconcile_codex_subagents_with(at,|_| {
            f.notify(json!({"method":"turn/started","params":{"threadId":"child","turnId":"new-B"}}));
            f.notify(json!({"method":"turn/completed","params":{"threadId":"child","turn":{"id":"old-A","status":"completed"}}}));
            Ok(value.clone())
        });
        assert_eq!(changed, 0);
        assert_eq!(f.child().status, "running");
        let active = json!({"thread":{"id":"child","status":{"type":"active","activeFlags":["waitingOnApproval"]},"source":{"subAgent":{"thread_spawn":{"parent_thread_id":"native-root"}}},"turns":[{"id":"old-A","status":"completed","startedAt":1},{"id":"new-B","status":"inProgress","startedAt":2}]}});
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + CHILD_INTERVAL, |_| Ok(active.clone())),
            0
        );
        // When the latest native turn actually fails, a known child
        // completion wakes the bounded read; it never completes the parent.
        f.notify(json!({"method":"thread/status/changed","params":{"threadId":"child","status":{"type":"notLoaded"}}}));
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + CHILD_INTERVAL * 2, |_| Ok(value.clone())),
            1
        );
        assert_eq!(f.child().status, "error");
    }

    #[test]
    fn subagent_notifications_reject_unrelated_envelopes_unknown_targets_and_old_owners() {
        let f = Fixture::new("notification-auth");
        let probe = f.probe();
        let frames = [
            json!({"method":"item/completed","params":{"threadId":"unrelated","turnId":"old","item":{"type":"collabAgentToolCall","receiverThreadIds":["child"],"agentsStates":{"child":{"status":"completed"}}}}}),
            json!({"method":"turn/completed","params":{"threadId":"unknown","turn":{"id":"turn","status":"failed"}}}),
            json!({"method":"item/completed","params":{"item":{"type":"subAgentActivity","agentThreadId":"child","kind":"completed"}}}),
        ];
        for frame in frames {
            f.notify(frame);
        }
        assert!(
            f.service
                .subagent_reconciliation
                .lock()
                .unwrap()
                .owners
                .is_empty()
        );
        assert_eq!(f.child().status, "running");
        f.service
            .runs
            .lock()
            .unwrap()
            .tasks
            .insert(f.task.clone(), RunControl::new(false));
        f.notify(json!({"method":"turn/completed","params":{"threadId":"child","turn":{"id":"turn","status":"failed"}}}));
        assert!(
            f.service
                .subagent_reconciliation
                .lock()
                .unwrap()
                .owners
                .is_empty()
        );
        assert!(!f.service.apply_subagent_read(&probe, &f.read()).unwrap());
    }

    #[test]
    fn subagent_polling_is_bounded_fair_and_retains_unsupported_suppression_while_busy() {
        let f = Fixture::new("fairness");
        let seed = f.probe();
        let mut candidates = Vec::new();
        let mut owners = HashMap::new();
        for index in 0..8 {
            let mut p = seed.clone();
            p.task_id = format!("task-{index}");
            p.control = RunControl::new(false);
            owners.insert(p.task_id.clone(), p.control.clone());
            candidates.push(p);
        }
        let mut state = ReconciliationState::default();
        let at = Instant::now();
        let first = state.select(candidates.clone(), at, &owners);
        assert_eq!(first.len(), MAX_PROBES);
        let next = state.select(candidates.clone(), at + Duration::from_secs(10), &owners);
        assert_eq!(next.len(), MAX_PROBES);
        let first_tasks = first
            .iter()
            .map(|p| p.task_id.clone())
            .collect::<HashSet<_>>();
        assert!(next.iter().all(|p| !first_tasks.contains(&p.task_id)));
        let probe = &first[0];
        state.outcome(probe, at, &Err("Method not found".into()));
        state.select(vec![], at + Duration::from_secs(20), &owners);
        assert!(
            !state
                .select(candidates.clone(), at + Duration::from_secs(1000), &owners)
                .iter()
                .any(|p| p.task_id == probe.task_id)
        );
        let mut replacement = probe.clone();
        replacement.control = RunControl::new(false);
        owners.insert(replacement.task_id.clone(), replacement.control.clone());
        assert_eq!(
            state
                .select(
                    vec![replacement.clone()],
                    at + Duration::from_secs(1001),
                    &owners
                )
                .len(),
            1
        );
    }

    #[test]
    fn subagent_polling_rechecks_each_selected_owner_before_rpc() {
        let f = Fixture::new("pre-rpc-fence");
        let agent = f.service.snapshot().unwrap().agents[0].clone();
        let task = f
            .service
            .create_task(CreateTaskInput {
                agent_id: agent.id,
                title: "Second owner".into(),
                native_session_id: Some("second-root".into()),
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        let second = RunControl::new(false);
        second.begin_run().unwrap();
        second.mark_resident();
        second.set_app_server_thread("second-root".into());
        second.mark_idle();
        f.service
            .runs
            .lock()
            .unwrap()
            .tasks
            .insert(task.id.clone(), second.clone());
        f.service
            .mutate(None, |s| {
                s.tasks.iter_mut().find(|t| t.id == task.id).unwrap().status = "completed".into();
                let mut child = s.subagent_sessions[0].clone();
                child.id = "codex:second-child".into();
                child.agent_thread_id = Some("second-child".into());
                child.parent_task_id = task.id.clone();
                s.subagent_sessions.push(child);
                Ok(())
            })
            .unwrap();
        let mut queried = Vec::new();
        let changed = f
            .service
            .reconcile_codex_subagents_with(Instant::now(), |probe| {
                queried.push(probe.task_id.clone());
                let (other_id, other) = if probe.task_id == f.task {
                    (&task.id, &second)
                } else {
                    (&f.task, &f.owner)
                };
                other.begin_run().unwrap();
                f.service
                    .mutate(None, |s| {
                        s.tasks
                            .iter_mut()
                            .find(|t| &t.id == other_id)
                            .unwrap()
                            .status = "running".into();
                        Ok(())
                    })
                    .unwrap();
                let mut value = f.read();
                value["thread"]["id"] = json!(probe.session.agent_thread_id);
                value["thread"]["source"]["subAgent"]["thread_spawn"]["parent_thread_id"] =
                    json!(probe.root);
                Ok(value)
            });
        assert_eq!(queried.len(), 1);
        assert_eq!(changed, 1);
        let snapshot = f.service.snapshot().unwrap();
        assert_eq!(
            snapshot
                .subagent_sessions
                .iter()
                .filter(|s| s.status == "running")
                .count(),
            1
        );
    }

    #[test]
    fn subagent_polling_transient_timeout_recovers_after_drainage_and_cooldown() {
        let f = Fixture::new("timeout");
        let at = Instant::now();
        let mut calls = 0;
        assert_eq!(
            f.service.reconcile_codex_subagents_with(at, |_| {
                calls += 1;
                Err("Codex app-server thread/read request timed out.".into())
            }),
            0
        );
        f.owner.begin_run().unwrap();
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + Duration::from_secs(30), |_| panic!(
                    "active root must not query"
                )),
            0
        );
        f.owner.mark_idle();
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + Duration::from_secs(60), |_| panic!(
                    "timeout cooldown must remain"
                )),
            0
        );
        assert_eq!(
            f.service
                .reconcile_codex_subagents_with(at + Duration::from_secs(121), |_| {
                    calls += 1;
                    Ok(f.read())
                }),
            1
        );
        assert_eq!(calls, 2);
        assert_eq!(f.child().status, "error");
    }

    #[test]
    fn subagent_completion_time_rejects_invalid_native_clock_values() {
        let at = now();
        assert_eq!(completion_time(&json!({"completedAt":i64::MAX}), 0), None);
        assert_eq!(completion_time(&json!({"completedAt":-1}), 0), None);
        assert_eq!(
            completion_time(&json!({"completedAt":at/1000-100}), at - 1000),
            None
        );
        assert_eq!(
            completion_time(&json!({"completedAt":at/1000+3600}), 0),
            None
        );
    }
}
