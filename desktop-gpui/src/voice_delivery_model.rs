//! Bounded, ordered voice admission. Only one mutation is outstanding, while
//! all accepted messages remain tracked until their canonical delivery exists.
use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Pending,
    Sending,
    Waiting,
}

struct Entry {
    id: String,
    text: String,
    stage: Stage,
}

#[derive(Default)]
pub struct VoiceDelivery {
    entries: VecDeque<Entry>,
    delivered_at: Option<i64>,
}

impl VoiceDelivery {
    pub fn enqueue(&mut self, id: String, text: String) -> bool {
        if self.entries.len() >= 4 || self.entries.iter().any(|entry| entry.id == id) {
            return false;
        }
        self.entries.push_back(Entry {
            id,
            text,
            stage: Stage::Pending,
        });
        true
    }

    pub fn next_submission(&mut self) -> Option<(String, String)> {
        if self
            .entries
            .iter()
            .any(|entry| entry.stage == Stage::Sending)
        {
            return None;
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.stage == Stage::Pending)?;
        entry.stage = Stage::Sending;
        Some((entry.id.clone(), entry.text.clone()))
    }

    pub fn accepted(&mut self, id: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            if entry.stage == Stage::Sending {
                entry.stage = Stage::Waiting;
            }
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }

    pub fn waiting_id(&self) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.stage == Stage::Waiting)
            .map(|entry| entry.id.as_str())
    }

    pub fn delivered(&mut self, id: &str, created_at: i64) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.id == id && entry.stage == Stage::Waiting)
        {
            self.entries.remove(index);
            self.delivered_at = Some(
                self.delivered_at
                    .map_or(created_at, |old| old.max(created_at)),
            );
        }
    }

    /// Consume the delivery boundary only once every recognized final in this
    /// batch has reached the chat. This does not assert provider acknowledgement.
    pub fn take_ready_boundary(&mut self) -> Option<i64> {
        if self.entries.is_empty() {
            self.delivered_at.take()
        } else {
            None
        }
    }

    pub fn review_text(&self) -> String {
        self.entries
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interjections_submit_in_order_and_wait_for_every_delivery() {
        let mut flow = VoiceDelivery::default();
        assert!(flow.enqueue("a".into(), "first correction".into()));
        assert_eq!(flow.next_submission().unwrap().0, "a");
        assert!(flow.enqueue("b".into(), "second correction".into()));
        assert!(flow.next_submission().is_none());
        flow.accepted("a");
        assert_eq!(flow.next_submission().unwrap().0, "b");
        flow.accepted("b");
        // The second message may reach a canonical row first, but it must not
        // release speech while the first message is still queued.
        flow.delivered("b", 102);
        assert!(flow.take_ready_boundary().is_none());
        assert_eq!(flow.waiting_id(), Some("a"));
        flow.delivered("a", 101);
        assert_eq!(flow.take_ready_boundary(), Some(102));
        assert!(flow.take_ready_boundary().is_none());
        // A stale poll cannot re-open or release a later admission batch.
        flow.accepted("b");
        flow.delivered("b", 999);
        assert!(flow.take_ready_boundary().is_none());
    }

    #[test]
    fn pending_final_holds_speech_and_capacity_is_bounded() {
        let mut flow = VoiceDelivery::default();
        for id in ["a", "b", "c", "d"] {
            assert!(flow.enqueue(id.into(), id.into()));
        }
        assert!(!flow.enqueue("e".into(), "e".into()));
        assert!(!flow.enqueue("a".into(), "changed".into()));
        flow.next_submission();
        flow.accepted("a");
        flow.delivered("a", 100);
        assert!(flow.take_ready_boundary().is_none());
        assert_eq!(flow.review_text(), "b\n\nc\n\nd");
        assert_eq!(flow.next_submission().unwrap().0, "b");
    }
}
