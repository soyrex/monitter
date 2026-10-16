//! Task-scoped, coalesced model and access intent. Only these idempotent setters
//! are reconciled; a single worker serializes both settings for each task.
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(super) fn normalized(settings: &Value) -> Value {
    json!({"model":settings["model"].as_str().unwrap_or_default(),
        "reasoningEffort":settings["reasoningEffort"].as_str(),
        "fastMode":settings["fastMode"].as_bool()})
}
pub(super) fn task_settings(snapshot: &Value, task_id: &str) -> Option<Value> {
    let task = snapshot["tasks"]
        .as_array()?
        .iter()
        .find(|task| task["id"].as_str() == Some(task_id))?;
    Some(if task["modelSettings"].is_object() {
        normalized(&task["modelSettings"])
    } else {
        normalized(&json!({"model":task["model"]}))
    })
}
pub(super) fn available(task: Option<&Value>) -> bool {
    task.is_some_and(|task| task["archived"].as_bool() != Some(true))
}
pub(super) fn recoverable(error: &str) -> bool {
    crate::chat::connection_recovery::transient(error)
        || matches!(
            error,
            "Experimental backend was restarted. Reconnect and refresh state before issuing commands."
        )
}
pub(super) fn deferred(error: &str) -> bool {
    error == "Wait for the current run to finish before changing permissions."
}
fn actual_settings(snapshot: &Value, task: &str, sent: &Value) -> Option<Value> {
    if sent.get("sandbox").is_some() {
        let task = snapshot["tasks"]
            .as_array()?
            .iter()
            .find(|value| value["id"].as_str() == Some(task))?;
        Some(json!({"sandbox": task["sandbox"]}))
    } else {
        task_settings(snapshot, task)
    }
}
#[derive(Default, Debug)]
pub(super) struct ModelSaveQueue {
    pub epoch: u64,
    pub desired: Option<Value>,
    pub sandbox: Option<Value>,
    pub working: bool,
    pub paused: bool,
    resume_at: Option<Instant>,
    pub attempts: u8,
    pub error: Option<String>,
}
impl ModelSaveQueue {
    pub fn clear(&mut self) {
        let epoch = self.epoch.wrapping_add(1);
        *self = Self {
            epoch,
            ..Self::default()
        };
    }
    pub fn enqueue(&mut self, settings: Value) {
        self.desired = Some(normalized(&settings));
        self.error = None;
        self.paused = false;
        self.resume_at = None;
        self.attempts = 0;
    }
    pub fn enqueue_sandbox(&mut self, sandbox: String) {
        self.sandbox = Some(json!({"sandbox": sandbox}));
        self.error = None;
        self.paused = false;
        self.resume_at = None;
        self.attempts = 0;
    }
    pub fn pending(&self) -> Option<Value> {
        self.desired.clone().or_else(|| self.sandbox.clone())
    }
    pub fn begin(&mut self) -> Option<Value> {
        if self.working || self.paused {
            return None;
        }
        let desired = self.pending()?;
        self.working = true;
        Some(desired)
    }
    pub fn saved(&mut self, sent: &Value) {
        self.working = false;
        self.attempts = 0;
        self.paused = false;
        self.resume_at = None;
        if self.sandbox.as_ref() == Some(sent) {
            self.sandbox = None;
        }
        if self.desired.as_ref() == Some(sent) {
            self.desired = None;
        }
    }
    pub fn rejected(&mut self, sent: &Value) {
        self.saved(sent);
    }
    pub fn interrupted(&mut self) {
        self.interrupted_at(Instant::now());
    }
    fn interrupted_at(&mut self, now: Instant) {
        self.working = false;
        self.attempts = self.attempts.saturating_add(1);
        self.paused = self.attempts >= 3;
        self.resume_at = self.paused.then_some(now + Duration::from_secs(2));
    }
    pub fn reconcile(&mut self, snapshot: &Value, task: &str) {
        self.reconcile_at(snapshot, task, Instant::now());
    }
    fn reconcile_at(&mut self, snapshot: &Value, task: &str, now: Instant) {
        if self.working {
            return;
        }
        if let Some(desired) = &self.sandbox {
            if actual_settings(snapshot, task, desired).as_ref() == Some(desired) {
                self.sandbox = None;
            }
        }
        if let Some(actual) = task_settings(snapshot, task) {
            if self.desired.as_ref() == Some(&actual) {
                self.desired = None;
            }
            // Equal snapshots are fresh authority too, but must not turn an
            // outage into an immediate repeated mutation loop.
            if self.pending().is_none()
                || self.paused && self.resume_at.is_none_or(|resume| now >= resume)
            {
                self.paused = false;
                self.resume_at = None;
                self.attempts = 0;
            }
        }
    }
}
pub(super) enum SaveOutcome {
    Saved(Value),
    Interrupted(Option<Value>),
    Deferred,
    Rejected(String),
}
/// A read can acknowledge a lost response. A mismatch retains intent for the
/// next serialized setter attempt, using the newest choice at that time.
pub(super) fn after_write(
    result: Result<Value, String>,
    task: &str,
    sent: &Value,
    read: impl FnOnce() -> Result<Value, String>,
) -> SaveOutcome {
    match result {
        Ok(snapshot) => SaveOutcome::Saved(snapshot),
        Err(error) if deferred(&error) => SaveOutcome::Deferred,
        Err(error) if recoverable(&error) => {
            let snapshot = read().ok().and_then(|value| {
                value
                    .get("snapshot")
                    .filter(|value| value.is_object())
                    .cloned()
            });
            if snapshot
                .as_ref()
                .and_then(|snapshot| actual_settings(snapshot, task, sent))
                .as_ref()
                == Some(sent)
            {
                SaveOutcome::Saved(snapshot.unwrap())
            } else {
                SaveOutcome::Interrupted(snapshot)
            }
        }
        Err(error) => SaveOutcome::Rejected(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const LOST: &str = "Private IPC connection lost; delivery may be ambiguous. Refresh state before retrying a mutation.";
    fn settings(model: &str) -> Value {
        normalized(&json!({"model":model}))
    }
    #[test]
    fn access_and_model_coalesce_independently_and_share_one_worker() {
        let mut queue = ModelSaveQueue::default();
        queue.enqueue_sandbox("read-only".into());
        let first = queue.begin().unwrap();
        queue.enqueue_sandbox("workspace-write".into());
        queue.enqueue_sandbox("yolo".into());
        queue.enqueue(settings("latest"));
        assert!(queue.begin().is_none());
        queue.saved(&first);
        assert_eq!(queue.begin(), Some(settings("latest")));
        queue.saved(&settings("latest"));
        assert_eq!(queue.begin(), Some(json!({"sandbox":"yolo"})));
        queue.saved(&json!({"sandbox":"yolo"}));
        assert!(queue.pending().is_none());
    }
    #[test]
    fn access_lost_ack_reconciles_exact_authority_and_busy_is_deferred() {
        let sent = json!({"sandbox":"yolo"});
        assert!(matches!(
            after_write(Err(LOST.into()), "a", &sent, || {
                Ok(json!({"snapshot":{"tasks":[{"id":"a","sandbox":"yolo"}]}}))
            }),
            SaveOutcome::Saved(_)
        ));
        assert!(matches!(
            after_write(
                Err("Wait for the current run to finish before changing permissions.".into()),
                "a",
                &sent,
                || panic!("busy is not a transport error")
            ),
            SaveOutcome::Deferred
        ));
        assert!(matches!(
            after_write(Err("Unsupported sandbox".into()), "a", &sent, || panic!(
                "unsupported must remain visible"
            )),
            SaveOutcome::Rejected(_)
        ));
        let mut queue = ModelSaveQueue::default();
        queue.enqueue_sandbox("yolo".into());
        queue.reconcile(&json!({"tasks":[{"id":"b","sandbox":"yolo"}]}), "a");
        assert_eq!(queue.sandbox, Some(sent.clone()));
        queue.reconcile(&json!({"tasks":[{"id":"a","sandbox":"yolo"}]}), "a");
        assert!(queue.sandbox.is_none());
    }
    #[test]
    fn identical_authority_resumes_after_cooldown_without_resetting_deadline() {
        let now = Instant::now();
        let actual = json!({"tasks":[{"id":"task","model":"old","sandbox":"read-only"}]});
        let mut queue = ModelSaveQueue::default();
        queue.enqueue_sandbox("yolo".into());
        for _ in 0..3 {
            assert!(queue.begin().is_some());
            queue.interrupted_at(now);
        }
        for elapsed in [0, 500, 1500, 1999] {
            queue.reconcile_at(&actual, "task", now + Duration::from_millis(elapsed));
            assert!(queue.begin().is_none());
        }
        queue.reconcile_at(&actual, "task", now + Duration::from_secs(2));
        assert_eq!(queue.begin(), Some(json!({"sandbox":"yolo"})));
    }
    #[test]
    fn rapid_choices_coalesce_and_old_ack_cannot_replace_latest() {
        let mut queue = ModelSaveQueue::default();
        queue.enqueue(settings("A"));
        let a = queue.begin().unwrap();
        queue.enqueue(settings("B"));
        queue.enqueue(settings("C"));
        assert!(queue.begin().is_none());
        queue.saved(&a);
        assert_eq!(queue.begin(), Some(settings("C")));
        queue.saved(&settings("C"));
        assert!(queue.desired.is_none());
    }
    #[test]
    fn lost_ack_is_confirmed_by_full_authoritative_settings() {
        let sent = normalized(&json!({"model":"A","reasoningEffort":"high","fastMode":true}));
        let read = || {
            Ok(json!({"revision":"r","snapshot":{"tasks":[{"id":"task","modelSettings":sent}]}}))
        };
        assert!(matches!(
            after_write(Err(LOST.into()), "task", &sent, read),
            SaveOutcome::Saved(_)
        ));
        let read = || {
            Ok(
                json!({"snapshot":{"tasks":[{"id":"task","modelSettings":{"model":"A","reasoningEffort":"low","fastMode":true}}]}}),
            )
        };
        assert!(matches!(
            after_write(Err(LOST.into()), "task", &sent, read),
            SaveOutcome::Interrupted(Some(_))
        ));
        assert!(matches!(
            after_write(Err("Unsupported model".into()), "task", &sent, || panic!(
                "semantic errors must not retry/read"
            )),
            SaveOutcome::Rejected(_)
        ));
    }
    #[test]
    fn reset_lost_ack_matches_authority_null_settings_and_empty_model() {
        let reset = settings("");
        let read =
            || Ok(json!({"snapshot":{"tasks":[{"id":"task","model":"","modelSettings":null}]}}));
        assert!(matches!(
            after_write(Err(LOST.into()), "task", &reset, read),
            SaveOutcome::Saved(_)
        ));
    }
    #[test]
    fn outage_retains_latest_effort_and_fast_and_valid_snapshot_resumes() {
        let mut queue = ModelSaveQueue::default();
        queue.enqueue(settings("A"));
        let a = queue.begin().unwrap();
        let latest = normalized(&json!({"model":"A","reasoningEffort":"high","fastMode":true}));
        queue.enqueue(latest.clone());
        queue.interrupted();
        assert_eq!(queue.begin(), Some(latest.clone()));
        queue.interrupted();
        assert_eq!(queue.begin(), Some(latest.clone()));
        queue.interrupted();
        assert!(queue.paused && queue.begin().is_none());
        queue.reconcile_at(
            &json!({"tasks":[{"id":"task","modelSettings":a}]}),
            "task",
            Instant::now() + Duration::from_secs(3),
        );
        assert_eq!(queue.begin(), Some(latest));
    }
    #[test]
    fn switching_tasks_invalidates_worker_and_semantic_error_preserves_newer_choice() {
        let mut queue = ModelSaveQueue::default();
        queue.enqueue(settings("A"));
        let a = queue.begin().unwrap();
        let old_epoch = queue.epoch;
        queue.enqueue(settings("B"));
        queue.rejected(&a);
        assert_eq!(queue.desired, Some(settings("B")));
        queue.clear();
        assert_ne!(queue.epoch, old_epoch);
        assert!(queue.begin().is_none());
        assert!(
            task_settings(&json!({"tasks":[{"id":"other","modelSettings":a}]}), "task").is_none()
        );
    }
    #[test]
    fn model_availability_ignores_runtime_busy_and_provider_but_rejects_archived_missing() {
        assert!(available(Some(
            &json!({"provider":"claude","status":"running","busy":true})
        )));
        assert!(!available(Some(&json!({"archived":true}))));
        assert!(!available(None));
    }
}
