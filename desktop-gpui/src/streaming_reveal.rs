//! Presentation-only reveal state. Source messages remain authoritative.
use std::collections::{HashMap, HashSet};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy)]
pub struct RevealMessage<'a> {
    pub id: &'a str,
    pub text: &'a str,
    pub is_streaming: bool,
    pub is_interrupted: bool,
}

#[derive(Default)]
pub struct RevealState {
    messages: HashMap<String, MessageReveal>,
}

struct MessageReveal {
    // One current snapshot per live message; no chunk/history accumulation.
    source: String,
    ends: Vec<usize>,
    visible: usize,
    streaming: bool,
}

impl MessageReveal {
    fn new(message: RevealMessage<'_>, fully_visible: bool) -> Self {
        Self {
            source: message.text.to_owned(),
            ends: grapheme_ends(message.text),
            visible: if fully_visible { message.text.len() } else { 0 },
            streaming: message.is_streaming && !message.is_interrupted,
        }
    }
    fn pending(&self) -> bool {
        self.visible < self.source.len()
    }
}

fn grapheme_ends(text: &str) -> Vec<usize> {
    text.grapheme_indices(true)
        .map(|(start, g)| start + g.len())
        .collect()
}

impl RevealState {
    /// Seed the current snapshot fully visible when mounting or enabling the effect.
    /// Existing streaming history is also visible; only subsequent appends animate.
    pub fn seed<'a>(&mut self, messages: impl IntoIterator<Item = RevealMessage<'a>>) {
        self.messages.clear();
        for message in messages {
            self.messages
                .insert(message.id.to_owned(), MessageReveal::new(message, true));
        }
    }

    /// Reconcile an authoritative snapshot and prune messages no longer present.
    pub fn sync<'a>(&mut self, messages: impl IntoIterator<Item = RevealMessage<'a>>) {
        let mut present = HashSet::new();
        for message in messages {
            present.insert(message.id.to_owned());
            let Some(state) = self.messages.get_mut(message.id) else {
                self.messages.insert(
                    message.id.to_owned(),
                    MessageReveal::new(message, !message.is_streaming || message.is_interrupted),
                );
                continue;
            };
            if state.source != message.text {
                let appended = message.text.starts_with(&state.source);
                state.source = message.text.to_owned();
                state.ends = grapheme_ends(message.text);
                if !appended || message.is_interrupted {
                    state.visible = message.text.len();
                } else {
                    // An appended combining mark/ZWJ can extend the last visible
                    // grapheme. Include its extension rather than split a cluster.
                    if state.visible > 0 {
                        if let Some(end) =
                            state.ends.iter().copied().find(|end| *end >= state.visible)
                        {
                            state.visible = end;
                        }
                    }
                }
            }
            if message.is_interrupted {
                state.visible = message.text.len();
            }
            state.streaming = message.is_streaming && !message.is_interrupted;
        }
        self.messages.retain(|id, _| present.contains(id));
    }

    /// Advance exactly one extended Unicode grapheme per pending message.
    /// Returns whether any visible prefix changed.
    pub fn tick(&mut self) -> bool {
        self.advance(1)
    }

    /// Advance a frame's budget without splitting any Unicode grapheme.
    pub fn advance(&mut self, characters: usize) -> bool {
        if characters == 0 {
            return false;
        }
        let mut changed = false;
        for state in self.messages.values_mut() {
            if state.pending() {
                let next = state.ends.partition_point(|end| *end <= state.visible);
                let end = next.saturating_add(characters).min(state.ends.len()) - 1;
                state.visible = state.ends[end];
                changed = true;
            }
        }
        changed
    }

    /// UTF-8 prefix length for a known message. Unknown IDs render their full source.
    pub fn visible_bytes(&self, id: &str) -> Option<usize> {
        self.messages.get(id).map(|state| state.visible)
    }

    /// Keep the streaming presentation until both generation and reveal have ended.
    pub fn cursor_visible(&self, id: &str) -> bool {
        self.messages
            .get(id)
            .is_some_and(|state| state.streaming || state.pending())
    }

    pub fn is_pending(&self, id: &str) -> bool {
        self.messages.get(id).is_some_and(MessageReveal::pending)
    }

    /// Schedule ticks only while there are unrevealed graphemes.
    pub fn has_pending(&self) -> bool {
        self.messages.values().any(MessageReveal::pending)
    }

    /// Includes streaming messages waiting for their next source chunk.
    pub fn is_active(&self) -> bool {
        self.messages
            .values()
            .any(|state| state.streaming || state.pending())
    }

    /// Disable/hide: discard all presentation state. Unknown IDs then render fully.
    /// Call `seed` with the current snapshot before enabling/showing again.
    pub fn flush(&mut self) {
        self.messages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(text: &str, streaming: bool) -> RevealMessage<'_> {
        RevealMessage {
            id: "m",
            text,
            is_streaming: streaming,
            is_interrupted: false,
        }
    }
    #[test]
    fn unicode_ticks_are_graphemes() {
        let mut state = RevealState::default();
        state.sync([message("e\u{301}👩‍💻🇦🇺!", true)]);
        for prefix in ["e\u{301}", "e\u{301}👩‍💻", "e\u{301}👩‍💻🇦🇺", "e\u{301}👩‍💻🇦🇺!"]
        {
            assert!(state.tick());
            assert_eq!(state.visible_bytes("m"), Some(prefix.len()));
        }
        assert!(!state.tick());
    }
    #[test]
    fn frame_budget_reveals_fast_without_splitting_unicode_or_overshooting() {
        let mut state = RevealState::default();
        state.sync([message("e\u{301}👩‍💻🇦🇺!", true)]);
        assert!(!state.advance(0));
        assert!(state.advance(3));
        assert_eq!(state.visible_bytes("m"), Some("e\u{301}👩‍💻🇦🇺".len()));
        assert!(state.advance(usize::MAX));
        assert_eq!(state.visible_bytes("m"), Some("e\u{301}👩‍💻🇦🇺!".len()));
        assert!(!state.has_pending());
        state.flush();
        let burst = "x".repeat(1800);
        state.sync([message(&burst, true)]);
        for _ in 0..60 { state.advance(30); }
        assert_eq!(state.visible_bytes("m"), Some(1800));
    }
    #[test]
    fn append_and_completion_drain() {
        let mut state = RevealState::default();
        state.sync([message("ab", true)]);
        state.tick();
        state.sync([message("abcd", false)]);
        assert_eq!(state.visible_bytes("m"), Some(1));
        assert!(state.cursor_visible("m"));
        for _ in 0..3 {
            state.tick();
        }
        assert!(!state.cursor_visible("m"));
        assert!(!state.has_pending());
    }
    #[test]
    fn seed_history_and_new_completed_messages_do_not_replay() {
        let mut state = RevealState::default();
        state.seed([message("history", true)]);
        assert_eq!(state.visible_bytes("m"), Some(7));
        assert!(!state.has_pending());
        state.sync([message("history!", true)]);
        assert_eq!(state.visible_bytes("m"), Some(7));
        state.tick();
        state.flush();
        state.sync([message("complete", false)]);
        assert_eq!(state.visible_bytes("m"), Some(8));
    }
    #[test]
    fn interruption_and_rewrite_flush() {
        let mut state = RevealState::default();
        state.sync([message("abc", true)]);
        state.sync([message("xyz", true)]);
        assert_eq!(state.visible_bytes("m"), Some(3));
        let mut interrupted = message("xyz more", true);
        interrupted.is_interrupted = true;
        state.sync([interrupted]);
        assert_eq!(state.visible_bytes("m"), Some(8));
        assert!(!state.is_active());
    }
    #[test]
    fn prune_and_flush_clear_activity() {
        let mut state = RevealState::default();
        state.sync([message("abc", true)]);
        state.sync([]);
        assert_eq!(state.visible_bytes("m"), None);
        assert!(!state.is_active());
        state.sync([message("abc", true)]);
        state.flush();
        assert_eq!(state.visible_bytes("m"), None);
        assert!(!state.has_pending());
    }
    #[test]
    fn appended_combining_mark_keeps_visible_prefix_valid() {
        let mut state = RevealState::default();
        state.sync([message("e", true)]);
        state.tick();
        state.sync([message("e\u{301}!", true)]);
        assert_eq!(state.visible_bytes("m"), Some("e\u{301}".len()));
        state.tick();
        assert_eq!(state.visible_bytes("m"), Some("e\u{301}!".len()));
    }
}
