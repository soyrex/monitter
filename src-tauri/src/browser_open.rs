//! Bounded, ephemeral requests to present agent output in native browser tabs.
use crate::{model::id, Service};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::VecDeque, time::{Duration, Instant}};
const MAX_RECEIPTS: usize = 64;
const RECEIPT_TTL: Duration = Duration::from_secs(30 * 60);
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserOpenRequest { pub id: String, pub task_id: String, pub url: String, pub title: Option<String> }
struct Receipt { request: BrowserOpenRequest, request_id: String, acknowledged: bool, touched: Instant, original_target: Option<String> }
#[derive(Default)]
pub(crate) struct BrowserOpenQueue(VecDeque<Receipt>);
impl BrowserOpenQueue {
    fn expire(&mut self, now: Instant) { self.0.retain(|r| now.saturating_duration_since(r.touched) < RECEIPT_TTL); }
    fn enqueue(&mut self, task_id: &str, request_id: &str, url: String, title: Option<String>, now: Instant) -> Result<BrowserOpenRequest, String> {
        self.expire(now);
        if let Some(r) = self.0.iter().find(|r| r.request.task_id == task_id && r.request_id == request_id) {
            if r.request.url != url || r.request.title != title { return Err("browser_open request_id already belongs to a different target or title.".into()); }
            return Ok(r.request.clone());
        }
        if self.0.len() == MAX_RECEIPTS {
            if let Some(index) = self.0.iter().position(|r| r.acknowledged) { self.0.remove(index); }
            else { return Err("Browser open queue is full; wait for the native desktop to consume requests.".into()); }
        }
        let request = BrowserOpenRequest { id: id(), task_id: task_id.into(), url, title };
        self.0.push_back(Receipt { request: request.clone(), request_id: request_id.into(), acknowledged: false, touched: now, original_target: None });
        Ok(request)
    }
    fn read(&mut self, now: Instant) -> Vec<BrowserOpenRequest> { self.expire(now); self.0.iter().filter(|r| !r.acknowledged).map(|r| r.request.clone()).collect() }
    fn ack(&mut self, id: &str, now: Instant) -> Result<(), String> {
        self.expire(now);
        let r = self.0.iter_mut().find(|r| r.request.id == id).ok_or("Browser open request was not found or expired.")?;
        if !r.acknowledged { r.acknowledged = true; r.touched = now; } Ok(())
    }
}
impl Service {
    pub(crate) fn browser_open_protocol(&self, caller_task: &str, request_id: &str, target: &str, title: Option<&str>) -> Result<Value, String> {
        if !cfg!(feature = "experimental-gpui") { return Err("browser_open requires the native GPUI desktop.".into()); }
        let (task, _) = self.require_collaboration_caller(caller_task)?;
        if request_id.trim().is_empty() || request_id.len() > 128 || request_id.chars().any(char::is_control) { return Err("browser_open request_id must be 1-128 bytes without control characters.".into()); }
        let title = title.map(str::trim).map(str::to_owned);
        if title.as_ref().is_some_and(|v| v.is_empty() || v.len() > 240 || v.chars().any(char::is_control)) { return Err("browser_open title must be 1-240 bytes without control characters.".into()); }
        let local = target.trim().starts_with('/') || tauri::Url::parse(target.trim()).is_ok_and(|url| url.scheme() == "file");
        if local {
            let data = self.data.lock().map_err(|_| "Monitter state lock failed.")?;
            if !data.snapshot.hosts.iter().any(|h| h.id == task.host_id && h.kind == "local") { return Err("Remote tasks must supply an HTTP(S) URL; their filesystem paths are not local desktop files.".into()); }
        }
        if target.len() > 8192 || target.chars().any(char::is_control) { return Err("Browser target is too long or contains control characters.".into()); }
        {
            let mut queue = self.browser_open_requests.lock().map_err(|_| "Browser open queue lock failed.")?;
            queue.expire(Instant::now());
            if let Some(receipt) = queue.0.iter().find(|receipt| receipt.request.task_id == caller_task && receipt.request_id == request_id) {
                if receipt.request.title != title { return Err("browser_open request_id already belongs to a different target or title.".into()); }
                if receipt.original_target.as_deref() == Some(target.trim()) || receipt.request.url == target.trim() {
                    return Ok(json!({"status":"queued", "id":receipt.request.id, "url":receipt.request.url}));
                }
            }
        }
        let url = crate::browser_links::normalize_target(target)?;
        let request = {
            let mut queue = self.browser_open_requests.lock().map_err(|_| "Browser open queue lock failed.")?;
            let request = queue.enqueue(caller_task, request_id, url, title, Instant::now())?;
            if let Some(receipt) = queue.0.iter_mut().find(|receipt| receipt.request.id == request.id) {
                if receipt.original_target.is_none() { receipt.original_target = Some(target.trim().into()); }
            }
            request
        };
        self.changed(Some(caller_task.into()));
        Ok(json!({"status":"queued", "id":request.id, "url":request.url}))
    }
    pub(crate) fn read_browser_open_requests(&self) -> Result<Vec<BrowserOpenRequest>, String> { Ok(self.browser_open_requests.lock().map_err(|_| "Browser open queue lock failed.")?.read(Instant::now())) }
    pub(crate) fn ack_browser_open_request(&self, id: &str) -> Result<(), String> { self.browser_open_requests.lock().map_err(|_| "Browser open queue lock failed.")?.ack(id, Instant::now()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_open_queue_deduplicates_until_expiry() {
        let now = Instant::now(); let mut q = BrowserOpenQueue::default();
        let one = q.enqueue("task", "retry", "https://example.com/".into(), None, now).unwrap();
        assert_eq!(q.read(now), vec![one.clone()]); assert_eq!(q.read(now), vec![one.clone()]);
        q.ack(&one.id, now).unwrap(); q.ack(&one.id, now).unwrap();
        assert!(q.read(now).is_empty());
        assert_eq!(q.enqueue("task", "retry", one.url.clone(), None, now).unwrap(), one);
        assert!(q.enqueue("task", "retry", "https://other.test/".into(), None, now).is_err());
        assert!(q.read(now + RECEIPT_TTL).is_empty());
        assert_ne!(q.enqueue("task", "retry", one.url.clone(), None, now + RECEIPT_TTL).unwrap().id, one.id);
    }
    #[test]
    fn browser_open_queue_caps_pending_and_reclaims_acknowledged() {
        let now = Instant::now(); let mut q = BrowserOpenQueue::default();
        for i in 0..MAX_RECEIPTS { q.enqueue("task", &i.to_string(), "https://example.com/".into(), None, now).unwrap(); }
        assert!(q.enqueue("task", "overflow", "https://example.com/".into(), None, now).is_err());
        let first = q.read(now)[0].id.clone(); q.ack(&first, now).unwrap();
        q.enqueue("task", "overflow", "https://example.com/".into(), None, now).unwrap();
        assert_eq!(q.read(now).len(), MAX_RECEIPTS);
    }
}
