//! Stop intent is accepted even while another command awaits acknowledgement.
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct StopRequests {
    pending: VecDeque<String>,
    active: Option<String>,
}
impl StopRequests {
    pub fn request(&mut self, task: String) {
        if self.active.as_ref() != Some(&task) && !self.pending.contains(&task) {
            self.pending.push_back(task);
        }
    }
    pub fn next(&mut self, busy: bool) -> Option<String> {
        if busy || self.active.is_some() {
            return None;
        }
        let task = self.pending.pop_front()?;
        self.active = Some(task.clone());
        Some(task)
    }
    pub fn complete(&mut self) {
        self.active = None;
    }
    pub fn contains(&self, task: &str) -> bool {
        self.active.as_deref() == Some(task) || self.pending.iter().any(|id| id == task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_requests_accept_while_busy_coalesce_and_keep_original_task() {
        let mut queue = StopRequests::default();
        queue.request("first".into());
        queue.request("first".into());
        assert!(queue.contains("first"));
        assert_eq!(queue.next(true), None);
        queue.request("second".into());
        assert_eq!(queue.next(false).as_deref(), Some("first"));
        queue.request("first".into());
        assert_eq!(queue.next(false), None);
        queue.complete();
        assert_eq!(queue.next(false).as_deref(), Some("second"));
        queue.complete();
        assert_eq!(queue.next(false), None);
    }
}
