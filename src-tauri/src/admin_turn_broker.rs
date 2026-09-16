//! Process-local broker for the resident Monitter Admin agent.
//!
//! Small interface "mini-LLM" turns route their prompts to a single long-running
//! task bound to the resident admin agent. The broker ensures that exactly one
//! admin turn is active at a time, that the only way to complete a registered
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
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex, Weak,
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
    closed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct AdminTurnBrokerState {
    current: Mutex<Option<AdminTurnRequest>>,
}

/// Process-local broker for the resident Monitter Admin agent's mini-LLM turns.
///
/// The broker is intentionally minimal: only one admin turn may be active at a
/// time, its completion callback is owned by a single [`Weak<RunControl>`],
/// and a timed-out request removes itself without invoking the callback.
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

    /// Returns the internal task id of the currently registered admin turn,
    /// if any.
    pub(crate) fn active_task_id(&self) -> Option<String> {
        self.state
            .current
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|request| request.task_id.clone()))
    }

    /// Register a new admin turn. Fails with a readable error if another turn
    /// is already registered. The returned [`Receiver`] waits for the broker
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
        let mut guard = self
            .state
            .current
            .lock()
            .map_err(|_| "Monitter Admin broker lock failed.".to_string())?;
        if let Some(existing) = guard.as_ref() {
            // The existing entry's owner must still be alive: a dropped
            // weak reference means the previous transport was lost and the
            // broker can accept a fresh registration.
            if existing.owner.strong_count() > 0 && !existing.closed {
                return Err(format!(
                    "Monitter Admin is busy with another interface request ({}).",
                    existing.task_id
                ));
            }
        }
        *guard = Some(AdminTurnRequest {
            request_id,
            task_id,
            owner,
            completion: tx,
            deadline,
            buffer: String::new(),
            closed: false,
        });
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
        let Some(request) = guard.as_mut() else {
            return false;
        };
        if request.task_id != task_id || request.closed {
            return false;
        }
        if request.owner.strong_count() == 0 {
            // The owning transport went away without finalising the broker;
            // remove the entry so the receiver observes a clean
            // disconnection rather than a stale callback.
            *guard = None;
            return false;
        }
        if request.buffer.len().saturating_add(delta.len()) > MAX_BUFFER_BYTES {
            // Drain the callback so the caller observes a clean error
            // rather than hanging on a forgotten channel.
            let _ = request.completion.send(AdminTurnReply::Error(
                "Monitter Admin reply exceeded the supported size.".into(),
            ));
            *guard = None;
            return false;
        }
        request.buffer.push_str(delta);
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
        let Some(request) = guard.as_mut() else {
            return false;
        };
        if request.task_id != task_id || request.closed {
            return false;
        }
        let text = std::mem::take(&mut request.buffer);
        let _ = request.completion.send(AdminTurnReply::Text(text));
        *guard = None;
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
        let Some(request) = guard.as_ref() else {
            return false;
        };
        if request.task_id != task_id {
            return false;
        }
        *guard = None;
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
        let Some(request) = guard.as_mut() else {
            return false;
        };
        if request.task_id != task_id || request.closed {
            return false;
        }
        let _ = request.completion.send(AdminTurnReply::Error(message));
        *guard = None;
        true
    }

    /// Cancel the active request and deliver [`AdminTurnReply::Timeout`].
    pub(crate) fn time_out(&self, task_id: &str) -> bool {
        let mut guard = match self.state.current.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(request) = guard.as_mut() else {
            return false;
        };
        if request.task_id != task_id || request.closed {
            return false;
        }
        let _ = request.completion.send(AdminTurnReply::Timeout);
        *guard = None;
        true
    }

    /// Returns a [`Weak`] reference suitable for the watchdog thread.
    pub(crate) fn downgrade(&self) -> Weak<AdminTurnBrokerState> {
        Arc::downgrade(&self.state)
    }

    /// Reset the broker; used by `Service::cleanup` so a final shutdown never
    /// leaves a registered sender dangling.
    pub(crate) fn reset(&self) {
        if let Ok(mut guard) = self.state.current.lock() {
            if let Some(request) = guard.as_mut() {
                request.closed = true;
                let _ = request.completion.send(AdminTurnReply::Error(
                    "Monitter Admin broker is shutting down.".into(),
                ));
            }
            *guard = None;
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
    owner: Weak<RunControl>,
    on_timeout: impl FnOnce() + Send + 'static,
) {
    thread::spawn(move || {
        let deadline = match broker.upgrade() {
            Some(state) => match state.current.lock() {
                Ok(guard) => match guard.as_ref() {
                    Some(request) if request.task_id == task_id => request.deadline,
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
                    if broker.time_out(&task_id) {
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
                    broker.cancel_silently(&task_id);
                }
                return;
            }
            // Poll the broker so a `complete()` or a dropped owner exits the
            // watchdog without waiting for the next tick.
            let still_active = match broker.upgrade() {
                Some(state) => match state.current.lock() {
                    Ok(guard) => match guard.as_ref() {
                        Some(request) if request.task_id == task_id && !request.closed => true,
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
    use std::sync::mpsc::RecvTimeoutError;

    fn fresh_request_id(label: &str) -> String {
        format!("req-{label}-{}", crate::id())
    }

    #[test]
    fn try_register_rejects_a_concurrent_request() {
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
        assert!(matches!(second, Err(message) if message.contains("Monitter Admin is busy")));
        drop(first);
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
        let receiver = broker
            .try_register(
                fresh_request_id("wd"),
                "task-watchdog".into(),
                Arc::downgrade(&owner),
                Instant::now() + Duration::from_millis(50),
            )
            .unwrap();
        let broker_weak = broker.downgrade();
        let owner_weak = Arc::downgrade(&owner);
        spawn_admin_turn_watchdog(broker_weak, "task-watchdog".into(), owner_weak, move || {
            // Timeout hook is intentionally empty in the test.
        });
        let reply = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("watchdog must deliver a reply before its own timeout");
        assert!(matches!(reply, AdminTurnReply::Timeout));
    }

    #[test]
    fn watchdog_exits_when_owner_drops() {
        let broker = AdminTurnBroker::new();
        let owner: Arc<RunControl> = RunControl::new(false);
        let receiver = broker
            .try_register(
                fresh_request_id("wd-drop"),
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
        let receiver = broker
            .try_register(
                fresh_request_id("wd-complete"),
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
