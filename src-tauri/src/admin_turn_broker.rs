//! Process-local broker for the resident Monitter Admin agent.
//!
//! Small interface "mini-LLM" turns route their prompts to one of three hidden,
//! long-running tasks bound to the resident admin agent. The broker ensures that
//! only one turn owns each task, that the only way to complete a registered
//! request is through the [`runner::RunControl`] that owns the resident task,
//! and that an accepted request will never replay a prompt after the task or
//! transport ends.
//!
//! The broker is intentionally process-local and runtime-only:
//! - It does not persist any state.
//! - It does not write messages or events to the snapshot.
//! - It does not survive a restart; a restart must never replay an in-flight
//!   request, and the next explicit mini-task starts a fresh resident transport.
//!
//! ## Correlation
//!
//! Each active admin turn carries:
//! - A random, locally-minted request id.
//! - The internal task id.
//! - A [`Weak`] handle to the owning [`runner::RunControl`]; the broker only
//!   releases the completion callback while that owner is still alive.
//! - A deadline used for the user-facing timeout.
//! - A process-local buffer holding the streamed assistant text so it can be
//!   returned to the caller without persisting it in a `Snapshot`.
//!
//! The completion callback is the *only* path that delivers text to the
//! caller. When the broker is asked to capture, complete, or cancel a request,
//! it rejects any operation that does not identify an active entry.

use std::{
    collections::HashMap,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Condvar, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

use crate::runner::RunControl;

/// The terminal outcome reported to the caller that registered an admin turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdminTurnReply {
    /// The resident admin transport finished normally and returned text.
    Text(String),
    /// The deadline elapsed before the resident transport finished.
    Timeout,
    /// The resident transport reported an error, the owner lost its claim,
    /// or the broker could not deliver the buffered text safely.
    Error(String),
}

#[derive(Debug)]
struct AdminTurnRequest {
    #[allow(dead_code)]
    request_id: String,
    task_id: String,
    owner: Weak<RunControl>,
    completion: Sender<AdminTurnReply>,
    deadline: Instant,
    buffer: String,
    // Full text last seen for each Codex app-server item. This is runtime
    // only and lets completed items replace their preceding deltas exactly.
    app_server_items: HashMap<String, String>,
    app_server_item_order: Vec<String>,
    app_server_prefix_len: Option<usize>,
    closed: bool,
}

const MAX_CONCURRENT_TURNS: usize = 3;

#[derive(Debug, Default)]
pub(crate) struct AdminTurnBrokerState {
    current: Mutex<HashMap<String, AdminTurnRequest>>,
    pool: Mutex<AdminTurnPoolState>,
    pool_changed: Condvar,
}

#[derive(Debug, Default)]
struct AdminTurnPoolState {
    next_ticket: u64,
    serving_ticket: u64,
    reserved: std::collections::HashSet<String>,
    shutting_down: bool,
}

/// A FIFO reservation for one invisible admin task. Dropping it only makes the
/// slot reusable after the caller has finished its terminal cleanup.
pub(crate) struct AdminTurnPermit {
    state: Arc<AdminTurnBrokerState>,
    task_id: String,
}

impl AdminTurnPermit {
    pub(crate) fn task_id(&self) -> &str { &self.task_id }
}

impl Drop for AdminTurnPermit {
    fn drop(&mut self) {
        if let Ok(mut pool) = self.state.pool.lock() {
            pool.reserved.remove(&self.task_id);
            self.state.pool_changed.notify_all();
        }
    }
}

/// Process-local broker for the resident Monitter Admin agent's mini-LLM turns.
///
/// The broker is intentionally minimal: up to three task-isolated admin turns
/// may be active, each completion callback is owned by its [`Weak<RunControl>`],
/// and a timed-out request removes only its own generation.
///
/// The state is shared through an internal [` `Arc`] so background watchdog
/// threads can hold a [`Weak`] reference and exit cleanly when the owning
/// [`Service`] drops.
#[derive(Debug, Default, Clone)]
pub(crate) struct AdminTurnBroker {
    state: Arc<AdminTurnBrokerState>,
}

impl AdminTurnBroker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Blocks in FIFO order until one of the supplied hidden task slots is
    /// available. Queue time deliberately does not start a model deadline.
    pub(crate) fn acquire_slot(&self, task_ids: &[String]) -> Result<AdminTurnPermit, String> {
        if task_ids.is_empty() { return Err("No Monitter Admin chat slots are configured.".into()); }
        let mut pool = self.state.pool.lock().map_err(|_| "Monitter Admin pool lock failed.".to_string())?;
        let ticket = pool.next_ticket;
        pool.next_ticket = pool.next_ticket.wrapping_add(1);
        loop {
            if pool.shutting_down { return Err("Monitter Admin broker is shutting down.".into()); }
            if pool.serving_ticket == ticket && pool.reserved.len() < MAX_CONCURRENT_TURNS {
                if let Some(task_id) = task_ids.iter().find(|id| !pool.reserved.contains(id.as_str())).cloned() {
                    pool.reserved.insert(task_id.clone());
                    pool.serving_ticket = pool.serving_ticket.wrapping_add(1);
                    self.state.pool_changed.notify_all();
                    return Ok(AdminTurnPermit { state: Arc::clone(&self.state), task_id });
                }
            }
            pool = self.state.pool_changed.wait(pool).map_err(|_| "Monitter Admin pool lock failed.".to_string())?;
        }
    }

    /// Returns one active internal task id, if any. Compatibility helper for
    /// old focused tests; production callers use task-scoped operations.
    pub(crate) fn active_task_id(&self) -> Option<String> {
        self.state
            .current
            .lock()
            .ok()
            .and_then(|guard| guard.values().next().map(|request| request.task_id.clone()))
    }

    pub(crate) fn is_active_task(&self, task_id: &str) -> bool {
        self.state.current.lock().is_ok_and(|guard| guard.contains_key(task_id))
    }

    /// Register a new admin turn. Fails with a readable error if the selected
    /// slot is already registered or shutdown won the admission race. The
    /// returned [`Receiver`] waits for the broker
    /// to deliver a final [`AdminTurnReply`]; a dropped receiver is allowed
    /// but never causes the broker to leak a buffered reply into the snapshot.
    pub(crate) fn try_register(
        &self,
        request_id: String,
        task_id: String,
        owner: Weak<RunControl>,
        deadline: Instant,
    ) -> Result<Receiver<AdminTurnReply>, String> {
        let (tx, rx) = mpsc::channel();
        // Keep pool -> request lock order consistent with reset so a waiter
        // admitted just before shutdown cannot register afterward.
        let pool = self.state.pool.lock()
            .map_err(|_| "Monitter Admin pool lock failed.".to_string())?;
        if pool.shutting_down { return Err("Monitter Admin broker is shutting down.".into()); }
        let mut guard = self
            .state
            .current
            .lock()
            .map_err(|_| "Monitter Admin broker lock failed.".to_string())?;
        guard.retain(|_, request| request.owner.strong_count() > 0 && !request.closed);
        if guard.contains_key(&task_id) {
            return Err(format!("Monitter Admin task is already handling an interface request ({task_id})."));
        }
        if guard.len() >= MAX_CONCURRENT_TURNS {
            return Err("Monitter Admin capacity is unavailable.".into());
        }
        guard.insert(task_id.clone(), AdminTurnRequest {
            request_id,
            task_id,
            owner,
            completion: tx,
            deadline,
            buffer: String::new(),
            app_server_items: HashMap::new(),
            app_server_item_order: Vec::new(),
            app_server_prefix_len: None,
            closed: false,
        });
        drop(guard);
        drop(pool);
        Ok(rx)
    }

    /// Capture streamed assistant text. Returns `false` when there is no
    /// matching active entry — late output from a timed-out or completed turn
    /// is intentionally dropped instead of mutating any snapshot record.
    pub(crate) fn capture_assistant_text(&self, task_id: &str, delta: &str) -> bool {
        let mut guard = match self.state.current.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(request) = guard.get_mut(task_id) else {
            return false;
        };
        if request.task_id != task_id || request.closed {
            return false;
        }
        if request.owner.strong_count() == 0 {
            // The owning transport went away without finalising the broker;
            // remove the entry so the receiver observes a clean
            // disconnection rather than a stale callback.
            guard.remove(task_id);
            return false;
        }
        if request.buffer.len().saturating_add(delta.len()) > MAX_BUFFER_BYTES {
            // Drain the callback so the caller observes a clean error
            // rather than hanging on a forgotten channel.
            let _ = request.completion.send(AdminTurnReply::Error(
                "Monitter Admin reply exceeded the supported size.".into(),
            ));
            guard.remove(task_id);
            return false;
        }
        request.buffer.push_str(delta);
        true
    }

    /// Capture the current full text for one Codex app-server item. Unlike
    /// raw harness deltas, Codex may emit both incremental text and a final
    /// completed-item snapshot; use the opaque item id to replace the
    /// authoritative text while preserving first-seen item order.
    pub(crate) fn capture_app_server_item(&self, task_id: &str, item_id: &str, text: &str) -> bool {
        let mut guard = match self.state.current.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(request) = guard.get_mut(task_id) else {
            return false;
        };
        if request.task_id != task_id || request.closed || request.owner.strong_count() == 0 {
            return false;
        }
        if !request.app_server_items.contains_key(item_id) {
            if request.app_server_item_order.len() >= 256 {
                let _ = request.completion.send(AdminTurnReply::Error(
                    "Monitter Admin reply contained too many message items.".into(),
                ));
                guard.remove(task_id);
                return false;
            }
            request
                .app_server_prefix_len
                .get_or_insert(request.buffer.len());
            request.app_server_item_order.push(item_id.into());
        }
        request.app_server_items.insert(item_id.into(), text.into());
        let prefix = request
            .app_server_prefix_len
            .unwrap_or(request.buffer.len());
        let item_bytes = request
            .app_server_item_order
            .iter()
            .filter_map(|id| request.app_server_items.get(id))
            .map(String::len)
            .sum::<usize>();
        if prefix.saturating_add(item_bytes) > MAX_BUFFER_BYTES {
            let _ = request.completion.send(AdminTurnReply::Error(
                "Monitter Admin reply exceeded the supported size.".into(),
            ));
            guard.remove(task_id);
            return false;
        }
        request.buffer.truncate(prefix);
        for id in &request.app_server_item_order {
            request
                .buffer
                .push_str(request.app_server_items.get(id).expect("registered item"));
        }
        true
    }

    /// Complete the active request with the buffered text. Idempotent: a
    /// second call is a no-op. Returns `true` when the broker delivered the
    /// reply, `false` when there was no matching entry.
    pub(crate) fn complete(&self, task_id: &str) -> bool {
        let mut guard = match self.state.current.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(request) = guard.get_mut(task_id) else {
            return false;
        };
        if request.task_id != task_id || request.closed {
            return false;
        }
        let text = std::mem::take(&mut request.buffer);
        let _ = request.completion.send(AdminTurnReply::Text(text));
        guard.remove(task_id);
        true
    }

    /// Cancel the active request without invoking the completion callback.
    /// Used when the owning transport was lost and we do not want a stale
    /// reply to leak back into the snapshot. The receiver observes a
    /// disconnected channel.
    pub(crate) fn cancel_silently(&self, task_id: &str) -> bool {
        let mut guard = match self.state.current.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(request) = guard.get(task_id) else {
            return false;
        };
        if request.task_id != task_id {
            return false;
        }
        guard.remove(task_id);
        true
    }

    /// Cancel the active request and deliver a [`AdminTurnReply::Error`] when
    /// the caller is still waiting. Returns whether an entry was removed.
    #[allow(dead_code)] // Currently exercised only from the unit tests.
    pub(crate) fn cancel_with_error(&self, task_id: &str, message: String) -> bool {
        let mut guard = match self.state.current.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(request) = guard.get_mut(task_id) else {
            return false;
        };
        if request.task_id != task_id || request.closed {
            return false;
        }
        let _ = request.completion.send(AdminTurnReply::Error(message));
        guard.remove(task_id);
        true
    }

    /// Cancel the active request and deliver [`AdminTurnReply::Timeout`].
    pub(crate) fn time_out(&self, task_id: &str) -> bool {
        let mut guard = match self.state.current.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(request) = guard.get_mut(task_id) else {
            return false;
        };
        if request.task_id != task_id || request.closed {
            return false;
        }
        let _ = request.completion.send(AdminTurnReply::Timeout);
        guard.remove(task_id);
        true
    }

    fn time_out_if_current(&self, task_id: &str, request_id: &str) -> bool {
        let mut guard = match self.state.current.lock() { Ok(guard) => guard, Err(_) => return false };
        let Some(request) = guard.get_mut(task_id) else { return false; };
        if request.request_id != request_id || request.closed { return false; }
        let _ = request.completion.send(AdminTurnReply::Timeout);
        guard.remove(task_id);
        true
    }

    fn cancel_silently_if_current(&self, task_id: &str, request_id: &str) -> bool {
        let mut guard = match self.state.current.lock() { Ok(guard) => guard, Err(_) => return false };
        if !guard.get(task_id).is_some_and(|request| request.request_id == request_id) { return false; }
        guard.remove(task_id);
        true
    }

    /// Returns a [`Weak`] reference suitable for the watchdog thread.
    pub(crate) fn downgrade(&self) -> Weak<AdminTurnBrokerState> {
        Arc::downgrade(&self.state)
    }

    /// Reset the broker; used by `Service::cleanup` so a final shutdown never
    /// leaves a registered sender dangling.
    pub(crate) fn reset(&self) {
        // Set the admission fence before draining requests. `try_register`
        // holds this pool lock while it obtains the request map.
        let Ok(mut pool) = self.state.pool.lock() else { return };
        pool.shutting_down = true;
        self.state.pool_changed.notify_all();
        if let Ok(mut guard) = self.state.current.lock() {
            for request in guard.values_mut() {
                request.closed = true;
                let _ = request.completion.send(AdminTurnReply::Error(
                    "Monitter Admin broker is shutting down.".into(),
                ));
            }
            guard.clear();
        }
    }
}

/// Maximum number of bytes the broker will buffer for one request before
/// declaring the reply invalid. The cap is intentionally below the 2 MiB
/// limit used for ordinary assistant messages so this lane can never sneak
/// an unbounded payload into memory.
pub(crate) const MAX_BUFFER_BYTES: usize = 64 * 1024;

/// Watchdog thread entry point. The watchdog polls the broker every
/// [`WATCHDOG_TICK`]; on the configured deadline it cancels the request with
/// [`AdminTurnReply::Timeout`] and invokes the timeout hook (the canonical
/// `Service` path uses it to terminate the owning resident transport).
///
/// The watchdog takes [`Weak<RunControl>`] so a dropped transport ends the
/// turn silently and never leaks a late reply into the snapshot.
pub(crate) fn spawn_admin_turn_watchdog(
    broker: Weak<AdminTurnBrokerState>,
    task_id: String,
    request_id: String,
    owner: Weak<RunControl>,
    on_timeout: impl FnOnce() + Send + 'static,
) {
    thread::spawn(move || {
        let deadline = match broker.upgrade() {
            Some(state) => match state.current.lock() {
                Ok(guard) => match guard.get(&task_id) {
                    Some(request) if request.request_id == request_id => request.deadline,
                    _ => return,
                },
                Err(_) => return,
            },
            None => return,
        };
        loop {
            let now = Instant::now();
            if now >= deadline {
                if let Some(state) = broker.upgrade() {
                    let broker = AdminTurnBroker { state };
                    if broker.time_out_if_current(&task_id, &request_id) {
                        on_timeout();
                    }
                }
                return;
            }
            // If the owner was dropped, close the entry silently so we do
            // not leak a buffered reply into the snapshot.
            if owner.strong_count() == 0 {
                if let Some(state) = broker.upgrade() {
                    let broker = AdminTurnBroker { state };
                    broker.cancel_silently_if_current(&task_id, &request_id);
                }
                return;
            }
            // Poll the broker so a `complete()` or a dropped owner exits the
            // watchdog without waiting for the next tick.
            let still_active = match broker.upgrade() {
                Some(state) => match state.current.lock() {
                    Ok(guard) => match guard.get(&task_id) {
                        Some(request) if request.request_id == request_id && !request.closed => true,
                        _ => false,
                    },
                    Err(_) => false,
                },
                None => false,
            };
            if !still_active {
                return;
            }
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(WATCHDOG_TICK);
            thread::sleep(wait);
        }
    });
}

/// Polling interval used by the admin turn watchdog. Short enough that a
/// dropped owner closes its entry on the next tick without waiting the full
/// deadline.
const WATCHDOG_TICK: Duration = Duration::from_millis(100);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc::RecvTimeoutError, mpsc};

    fn fresh_request_id(label: &str) -> String {
        format!("req-{label}-{}", crate::id())
    }

    #[test]
    fn try_register_accepts_three_distinct_tasks_and_bounds_the_fourth() {
        let broker = AdminTurnBroker::new();
        let owner_a: Arc<RunControl> = RunControl::new(false);
        let owner_b: Arc<RunControl> = RunControl::new(false);
        let now = Instant::now();
        let first = broker
            .try_register(
                fresh_request_id("a"),
                "task-a".into(),
                Arc::downgrade(&owner_a),
                now + Duration::from_secs(5),
            )
            .expect("first registration must succeed");
        let second = broker.try_register(
            fresh_request_id("b"),
            "task-b".into(),
            Arc::downgrade(&owner_b),
            now + Duration::from_secs(5),
        );
        assert!(second.is_ok());
        let owner_c: Arc<RunControl> = RunControl::new(false);
        assert!(broker.try_register(fresh_request_id("c"), "task-c".into(), Arc::downgrade(&owner_c), now + Duration::from_secs(5)).is_ok());
        let owner_d: Arc<RunControl> = RunControl::new(false);
        assert!(matches!(broker.try_register(fresh_request_id("d"), "task-d".into(), Arc::downgrade(&owner_d), now + Duration::from_secs(5)), Err(message) if message.contains("capacity")));
        drop(first);
    }

    #[test]
    fn fifo_pool_queues_the_fourth_turn_until_a_slot_drains() {
        let broker = AdminTurnBroker::new();
        let tasks = vec!["slot-0".into(), "slot-1".into(), "slot-2".into()];
        let first = broker.acquire_slot(&tasks).unwrap();
        let second = broker.acquire_slot(&tasks).unwrap();
        let third = broker.acquire_slot(&tasks).unwrap();
        let waiting = broker.clone();
        let waiting_tasks = tasks.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || { tx.send(waiting.acquire_slot(&waiting_tasks).map(|permit| permit.task_id().to_string())).unwrap(); });
        assert!(matches!(rx.recv_timeout(Duration::from_millis(30)), Err(RecvTimeoutError::Timeout)));
        let released = first.task_id().to_string();
        drop(first);
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(), released);
        drop(second);
        drop(third);
    }

    #[test]
    fn reset_wakes_queued_pool_callers() {
        let broker = AdminTurnBroker::new();
        let tasks = vec!["slot-0".into(), "slot-1".into(), "slot-2".into()];
        let _a = broker.acquire_slot(&tasks).unwrap();
        let _b = broker.acquire_slot(&tasks).unwrap();
        let _c = broker.acquire_slot(&tasks).unwrap();
        let waiting = broker.clone();
        let waiting_tasks = tasks.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || { tx.send(waiting.acquire_slot(&waiting_tasks).err().unwrap()).unwrap(); });
        broker.reset();
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap().contains("shutting down"));
    }

    #[test]
    fn capture_assistant_text_appends_only_for_the_active_request() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let now = Instant::now();
        let receiver = broker
            .try_register(
                fresh_request_id("capture"),
                "task-capture".into(),
                Arc::downgrade(&owner),
                now + Duration::from_secs(5),
            )
            .unwrap();
        assert!(broker.capture_assistant_text("task-capture", "hello "));
        assert!(broker.capture_assistant_text("task-capture", "world"));
        // Foreign task_id must never leak into the active buffer.
        assert!(!broker.capture_assistant_text("task-other", "leak"));
        assert!(broker.complete("task-capture"));
        // Late captures after complete must be ignored, not delivered.
        assert!(!broker.capture_assistant_text("task-capture", "late"));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            AdminTurnReply::Text("hello world".into())
        );
    }

    #[test]
    fn codex_item_snapshots_deduplicate_by_item_without_dropping_repeated_deltas() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let receiver = broker
            .try_register(
                fresh_request_id("items"),
                "task-items".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        // Raw harness output must preserve repeated tokens.
        assert!(broker.capture_assistant_text("task-items", "ha"));
        assert!(broker.capture_assistant_text("task-items", "ha"));
        // Codex sends cumulative snapshots for the same item, then a second item.
        assert!(broker.capture_app_server_item("task-items", "one", " first"));
        assert!(broker.capture_app_server_item("task-items", "one", " first item"));
        assert!(broker.capture_app_server_item("task-items", "one", " first item"));
        assert!(broker.capture_app_server_item("task-items", "two", " second"));
        // A later authoritative completion can shorten/rewrite an earlier
        // item after another item exists; it replaces in place, not appends.
        assert!(broker.capture_app_server_item("task-items", "one", " first"));
        assert!(broker.complete("task-items"));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            AdminTurnReply::Text("haha first second".into())
        );
    }

    #[test]
    fn complete_is_idempotent() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let receiver = broker
            .try_register(
                fresh_request_id("idem"),
                "task-idem".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        broker.capture_assistant_text("task-idem", "once");
        assert!(broker.complete("task-idem"));
        assert!(!broker.complete("task-idem"));
        let reply = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(reply, AdminTurnReply::Text("once".into()));
    }

    #[test]
    fn cancel_silently_drops_the_buffer_without_invoking_the_callback() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let receiver = broker
            .try_register(
                fresh_request_id("silent"),
                "task-silent".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        broker.capture_assistant_text("task-silent", "would-have-leaked");
        assert!(broker.cancel_silently("task-silent"));
        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Disconnected)
        );
        // A second cancel must be a no-op, not a duplicate error.
        assert!(!broker.cancel_silently("task-silent"));
    }

    #[test]
    fn cancel_with_error_delivers_an_error_reply() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let receiver = broker
            .try_register(
                fresh_request_id("error"),
                "task-error".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        assert!(broker.cancel_with_error("task-error", "boom".into()));
        let reply = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(reply, AdminTurnReply::Error(message) if message == "boom"));
    }

    #[test]
    fn oversized_reply_is_rejected_without_persisting_text() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let receiver = broker
            .try_register(
                fresh_request_id("oversize"),
                "task-oversize".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let huge = "x".repeat(MAX_BUFFER_BYTES);
        assert!(broker.capture_assistant_text("task-oversize", &huge));
        // The next capture must close the entry with an error rather than
        // pushing more text.
        assert!(!broker.capture_assistant_text("task-oversize", "more"));
        let reply = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(reply, AdminTurnReply::Error(_)));
    }

    #[test]
    fn captured_text_for_an_unknown_task_is_dropped_silently() {
        let broker = AdminTurnBroker::new();
        assert!(!broker.capture_assistant_text("ghost", "anything"));
        assert!(!broker.complete("ghost"));
        assert!(!broker.cancel_silently("ghost"));
        assert!(!broker.cancel_with_error("ghost", "anything".into()));
    }

    #[test]
    fn lost_owner_closes_the_buffer_without_leaking_text() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let receiver = broker
            .try_register(
                fresh_request_id("lost"),
                "task-lost".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        // The strong owner is dropped, leaving the Weak handle dangling.
        drop(owner);
        assert!(!broker.capture_assistant_text("task-lost", "stale"));
        assert!(!broker.complete("task-lost"));
        let reply = receiver.recv_timeout(Duration::from_millis(50));
        // The receiver must observe a disconnected channel: dropping the
        // owner clears the broker entry so we never leak buffered text.
        assert!(matches!(reply, Err(RecvTimeoutError::Disconnected)));
    }

    #[test]
    fn watchdog_times_out_an_inactive_request() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let request_id = fresh_request_id("wd");
        let receiver = broker
            .try_register(
                request_id.clone(),
                "task-watchdog".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_millis(50),
            )
            .unwrap();
        let broker_weak = broker.downgrade();
        let owner_weak = Arc::downgrade(&owner);
        spawn_admin_turn_watchdog(broker_weak, "task-watchdog".into(), request_id, owner_weak, move || {
            // Timeout hook is intentionally empty in the test.
        });
        let reply = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("watchdog must deliver a reply before its own timeout");
        assert!(matches!(reply, AdminTurnReply::Timeout));
    }

    #[test]
    fn stale_watchdog_cannot_timeout_a_new_generation_on_the_same_task() {
        let broker = AdminTurnBroker::new();
        let owner_old: Arc<RunControl> = RunControl::new(false);
        let old_id = fresh_request_id("old");
        let old = broker.try_register(old_id.clone(), "task-reused".into(), Arc::downgrade(&owner_old), Instant::now() + Duration::from_millis(50)).unwrap();
        spawn_admin_turn_watchdog(broker.downgrade(), "task-reused".into(), old_id, Arc::downgrade(&owner_old), move || {});
        assert!(broker.complete("task-reused"));
        assert!(matches!(old.recv_timeout(Duration::from_secs(1)), Ok(AdminTurnReply::Text(_))));
        let owner_new: Arc<RunControl> = RunControl::new(false);
        let new_id = fresh_request_id("new");
        let fresh = broker.try_register(new_id, "task-reused".into(), Arc::downgrade(&owner_new), Instant::now() + Duration::from_secs(5)).unwrap();
        thread::sleep(Duration::from_millis(120));
        assert!(broker.capture_assistant_text("task-reused", "fresh"));
        assert!(broker.complete("task-reused"));
        assert_eq!(fresh.recv_timeout(Duration::from_secs(1)).unwrap(), AdminTurnReply::Text("fresh".into()));
    }

    #[test]
    fn watchdog_exits_when_owner_drops() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let request_id = fresh_request_id("wd-drop");
        let receiver = broker
            .try_register(
                request_id.clone(),
                "task-watchdog-drop".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
        let broker_weak = broker.downgrade();
        let owner_weak = Arc::downgrade(&owner);
        spawn_admin_turn_watchdog(
            broker_weak,
            "task-watchdog-drop".into(),
            request_id,
            owner_weak,
            move || {},
        );
        // Drop the owner and the watchdog must close the entry silently.
        drop(owner);
        thread::sleep(Duration::from_millis(500));
        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn reset_drops_the_active_request_with_an_error() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let receiver = broker
            .try_register(
                fresh_request_id("reset"),
                "task-reset".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        broker.reset();
        let reply = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(reply, AdminTurnReply::Error(_)));
        assert!(broker.active_task_id().is_none());
    }

    #[test]
    fn watchdog_completes_via_capture_then_complete_path() {
        // The watchdog must not interfere when the broker is finalised
        // through the normal capture/complete flow before the deadline.
        // `on_timeout` is the timeout hook and must never fire on the
        // happy path.
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let request_id = fresh_request_id("wd-complete");
        let receiver = broker
            .try_register(
                request_id.clone(),
                "task-watchdog-complete".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let broker_weak = broker.downgrade();
        let owner_weak = Arc::downgrade(&owner);
        let timeout_fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let timeout_flag = Arc::clone(&timeout_fired);
        spawn_admin_turn_watchdog(
            broker_weak,
            "task-watchdog-complete".into(),
            request_id,
            owner_weak,
            move || {
                timeout_flag.store(true, std::sync::atomic::Ordering::Release);
            },
        );
        broker.capture_assistant_text("task-watchdog-complete", "ok");
        assert!(broker.complete("task-watchdog-complete"));
        let reply = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(reply, AdminTurnReply::Text("ok".into()));
        // Give the watchdog a tick to observe the closure.
        thread::sleep(Duration::from_millis(150));
        assert!(
            !timeout_fired.load(std::sync::atomic::Ordering::Acquire),
            "watchdog must not call on_timeout on a clean reply"
        );
    }
}
