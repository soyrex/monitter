//! Bounded, process-local owner timeline for real resident Admin requests.
use serde::Serialize;
use std::{collections::VecDeque, sync::Mutex};

const MAX_ENTRIES: usize = 200;
const MAX_DETAIL_CHARS: usize = 2048;
#[derive(Clone, Debug, Default)]
pub(crate) struct AdminActivityOrigin {
    pub purpose: &'static str,
    pub task_id: Option<String>,
    pub pane_title: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminActivityEntry {
    pub id: String,
    pub purpose: String,
    pub task_id: Option<String>,
    pub pane_title: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub detail: String,
}
#[derive(Debug, Default)]
pub(crate) struct AdminActivityLog(Mutex<VecDeque<AdminActivityEntry>>);
fn bounded(value: &str) -> String {
    let mut chars = value.chars();
    let mut result: String = chars.by_ref().take(MAX_DETAIL_CHARS).collect();
    if chars.next().is_some() { result.push('…'); }
    result
}
pub(crate) fn sensitive(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    ["api_key", "api key", "access_token", "access token", "refresh_token", "password", "bearer ", "private key", "-----begin", "sk-", "[redacted_secret]"].iter().any(|marker| value.contains(marker))
}
fn safe_detail(value: &str) -> String {
    if sensitive(value) { "Sensitive response or error details omitted.".into() } else { bounded(value) }
}
pub(crate) fn failure_detail(error: &str) -> &'static str {
    if error.contains("45-second interface deadline") { "Admin exceeded its 45-second interface deadline." }
    else if error.contains("not configured") || error.contains("Multiple Monitter Admin") || error.contains("agent was lost") { "Monitter Admin configuration is unavailable or invalid." }
    else if error.contains("shutting down") { "Admin admission stopped because the backend is shutting down." }
    else if error.contains("prompt cannot be empty") { "Admin request was rejected because its prompt was empty." }
    else if error.contains("transport") || error.contains("closed before replying") { "The Admin transport failed or closed before returning a response." }
    else if error.contains("lock failed") { "Admin request failed because a backend state lock was unavailable." }
    else { "Admin request failed; no successful response was returned." }
}
impl AdminActivityLog {
    pub(crate) fn begin(&self, origin: AdminActivityOrigin, model: Option<String>) -> Result<String, String> {
        let mut entries = self.0.lock().map_err(|_| "Monitter Admin activity lock failed.")?;
        if entries.len() >= MAX_ENTRIES {
            if let Some(index) = entries.iter().position(|entry| entry.finished_at.is_some()) {
                entries.remove(index);
            } else {
                // Preserve every queued/running request. Refuse new admission
                // rather than silently lose an in-flight activity identity.
                return Err("Monitter Admin activity capacity reached; wait for an active request to finish.".into());
            }
        }
        let id = crate::id();
        entries.push_back(AdminActivityEntry {
            id: id.clone(), purpose: if origin.purpose.is_empty() { "admin_turn".into() } else { origin.purpose.into() },
            task_id: origin.task_id, pane_title: origin.pane_title.map(|title| safe_detail(&title)),
            model, status: "queued".into(), created_at: crate::now(), started_at: None,
            finished_at: None, detail: "Waiting for a Monitter Admin slot.".into(),
        });
        Ok(id)
    }
    pub(crate) fn running(&self, id: &str, model: Option<String>) {
        if let Ok(mut entries) = self.0.lock() {
            if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id && entry.finished_at.is_none()) {
                entry.status = "running".into(); entry.started_at = Some(crate::now());
                entry.model = model; entry.detail = "Generating an Admin response.".into();
            }
        }
    }
    pub(crate) fn finish(&self, id: &str, status: &str, detail: &str) {
        if let Ok(mut entries) = self.0.lock() {
            if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id && entry.finished_at.is_none()) {
                entry.status = status.into(); entry.finished_at = Some(crate::now()); entry.detail = safe_detail(detail);
            }
        }
    }
    pub(crate) fn read(&self) -> Result<Vec<AdminActivityEntry>, String> {
        self.0.lock().map(|entries| entries.iter().rev().cloned().collect())
            .map_err(|_| "Monitter Admin activity lock failed.".into())
    }
}
impl crate::Service {
    pub(crate) fn admin_activity_origin(&self, purpose: &'static str, task_id: Option<&str>, pane_title: Option<String>) -> AdminActivityOrigin {
        let task = task_id.and_then(|task_id| self.data.lock().ok().and_then(|data| data.snapshot.tasks.iter().find(|task| task.id == task_id).cloned()));
        AdminActivityOrigin { purpose, task_id: task.as_ref().map(|task| task.id.clone()), pane_title: pane_title.or_else(|| task.map(|task| task.title)) }
    }
    pub(crate) fn admin_activity_model(&self, hidden_task_id: Option<&str>) -> Option<String> {
        let configured = hidden_task_id.and_then(|id| self.data.lock().ok().and_then(|data| data.snapshot.tasks.iter().find(|task| task.id == id).map(|task| task.model_settings.as_ref().map(|settings| settings.model.clone()).filter(|model| !model.trim().is_empty()).unwrap_or_else(|| task.model.clone()))))
            .or_else(|| self.internal_admin().ok().map(|admin| admin.model));
        configured.map(|model| if model.trim().is_empty() || model.trim() == "auto" {
            "Harness default (configured)".into()
        } else { format!("Configured: {}", bounded(&model)) })
    }
    pub(crate) fn read_admin_activity(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"entries":self.admin_activity.read()?}))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admin_activity_tracks_queue_running_outcomes_and_newest_first() {
        let log = AdminActivityLog::default();
        let first = log.begin(AdminActivityOrigin {purpose:"autoname",task_id:Some("chat".into()),pane_title:Some("A pane".into())}, None).unwrap();
        assert_eq!(log.read().unwrap()[0].status, "queued");
        log.running(&first, Some("Configured: test-model".into()));
        log.finish(&first, "succeeded", "Generated response: A useful title");
        let second = log.begin(AdminActivityOrigin::default(), None).unwrap();
        log.finish(&second, "timeout", "Deadline exceeded.");
        let entries = log.read().unwrap();
        assert_eq!(entries[0].id, second); assert_eq!(entries[0].status, "timeout");
        assert_eq!(entries[1].task_id.as_deref(), Some("chat"));
        assert!(entries[1].started_at.is_some()); assert!(entries[1].finished_at.is_some());
        log.finish(&first, "failed", "Late result");
        assert_eq!(log.read().unwrap()[1].status, "succeeded");
    }
    #[test]
    fn admin_activity_preserves_active_entries_and_caps_admission() {
        let log = AdminActivityLog::default();
        let active = log.begin(AdminActivityOrigin::default(), None).unwrap();
        for _ in 1..MAX_ENTRIES { let id=log.begin(AdminActivityOrigin::default(),None).unwrap(); log.finish(&id,"failed","Admission failed."); }
        log.begin(AdminActivityOrigin::default(), None).unwrap();
        assert_eq!(log.read().unwrap().len(), MAX_ENTRIES);
        assert!(log.read().unwrap().iter().any(|entry| entry.id == active));
        let all_active=AdminActivityLog::default();
        for _ in 0..MAX_ENTRIES { all_active.begin(AdminActivityOrigin::default(), None).unwrap(); }
        assert!(all_active.begin(AdminActivityOrigin::default(), None).is_err());
        assert_eq!(all_active.read().unwrap().len(), MAX_ENTRIES);
    }
    #[test]
    fn admin_activity_output_is_bounded_and_sensitive_details_omitted() {
        assert!(safe_detail(&"界".repeat(3000)).chars().count() <= MAX_DETAIL_CHARS + 1);
        assert_eq!(safe_detail("Bearer secret-value"), "Sensitive response or error details omitted.");
        assert!(sensitive("API_KEY=secret-value"));
    }
}
