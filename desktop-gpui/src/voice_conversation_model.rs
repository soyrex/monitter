//! Device-local conversation policy. No microphone, IPC, task cancellation or
//! model calls live here; the native controller executes the returned decisions.
//! This module deliberately builds with `rustc --test` for model-free testing.

use std::collections::{HashMap, HashSet, VecDeque};

pub const MAX_TRANSCRIPT_CHARS: usize = 8_000;
pub const MAX_SPEECH_CHARS: usize = 24_000;
const MAX_SEGMENT_CHARS: usize = 320;
const MAX_MESSAGES: usize = 256;
const MAX_UTTERANCES: usize = 256;
const FALSE_CUE_HOLD_MS: u64 = 1_800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDisposition {
    Partial,
    Duplicate,
    Ignored,
    Backchannel,
    StopSpeaking,
    Submit,
    Review,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechSegment {
    pub message_id: String,
    pub generation: u64,
    pub text: String,
}

/// Only canonical, direct assistant text is eligible. The caller must derive
/// these fields from the selected task's authoritative transcript, not events,
/// rendered Markdown, tool output, peer messages or an optimistic local echo.
pub struct AssistantText<'a> {
    pub id: &'a str,
    pub text: &'a str,
    pub created_at: i64,
    pub direct_assistant: bool,
    pub visible_phase: bool,
    pub complete: bool,
    pub interrupted: bool,
}

#[derive(Default)]
struct MessageCursor {
    text: String,
    consumed: usize,
    blocked: bool,
}

#[derive(Default)]
struct UtteranceCursor {
    revision: u64,
    final_seen: bool,
}

struct HeldMessage {
    text: String,
    created_at: i64,
    complete: bool,
}

/// Bounded, in-memory playback and recognition policy. Durable exactly-once
/// admission is a separate backend responsibility; this only avoids duplicate
/// local work and never retries an uncertain mutation.
pub struct ConversationModel {
    active: bool,
    muted: bool,
    started_at: i64,
    generation: u64,
    messages: HashMap<String, MessageCursor>,
    baseline: HashSet<String>,
    utterances: HashMap<String, UtteranceCursor>,
    queue: VecDeque<SpeechSegment>,
    queued_chars: usize,
    current: Option<SpeechSegment>,
    paused_at: Option<u64>,
    input_active: bool,
    input_changed_at: u64,
    waiting_for_admission: bool,
    held: HashMap<String, HeldMessage>,
    pub partial_text: String,
    pub last_final_text: String,
}

impl Default for ConversationModel {
    fn default() -> Self {
        Self {
            active: false,
            muted: false,
            started_at: 0,
            generation: 0,
            messages: HashMap::new(),
            baseline: HashSet::new(),
            utterances: HashMap::new(),
            queue: VecDeque::new(),
            queued_chars: 0,
            current: None,
            paused_at: None,
            input_active: false,
            input_changed_at: 0,
            waiting_for_admission: false,
            held: HashMap::new(),
            partial_text: String::new(),
            last_final_text: String::new(),
        }
    }
}

impl ConversationModel {
    pub fn begin(&mut self, now: i64, existing_ids: impl IntoIterator<Item = String>) {
        self.end();
        self.active = true;
        self.started_at = now;
        // IDs already visible at the explicit Start action are never read aloud.
        // Keep only the newest bounded tail while consuming the iterator. A
        // task history can be large, so collecting it before truncating would
        // temporarily duplicate every message ID at voice-session startup.
        let mut tail = VecDeque::with_capacity(512);
        for id in existing_ids {
            if tail.len() == 512 {
                tail.pop_front();
            }
            tail.push_back(id);
        }
        self.baseline = tail.into_iter().collect();
    }

    pub fn end(&mut self) {
        self.active = false;
        self.muted = false;
        self.cancel_speech();
        self.messages.clear();
        self.baseline.clear();
        self.utterances.clear();
        self.partial_text.clear();
        self.last_final_text.clear();
        self.input_active = false;
        self.waiting_for_admission = false;
    }

    pub fn active(&self) -> bool {
        self.active
    }
    pub fn muted(&self) -> bool {
        self.muted
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn speaking(&self) -> bool {
        self.current.is_some()
    }
    pub fn paused(&self) -> bool {
        self.paused_at.is_some()
    }
    pub fn current(&self) -> Option<&SpeechSegment> {
        self.current.as_ref()
    }

    /// While an utterance is in flight or queued, do not read the agent's older
    /// response over it. Delivery only establishes a new playback boundary; it
    /// cannot prove that subsequent words semantically address an interjection.
    pub fn await_admission(&mut self) {
        self.waiting_for_admission = true;
    }
    pub fn admission_delivered(&mut self, message_created_at: i64) -> bool {
        self.waiting_for_admission = false;
        let mut held: Vec<_> = self.held.drain().collect();
        held.sort_by(|(a_id, a), (b_id, b)| (a.created_at, a_id).cmp(&(b.created_at, b_id)));
        for (id, row) in held {
            if row.created_at < message_created_at {
                self.messages.insert(
                    id,
                    MessageCursor {
                        blocked: true,
                        ..Default::default()
                    },
                );
                continue;
            }
            if !self.observe(AssistantText {
                id: &id,
                text: &row.text,
                created_at: row.created_at,
                direct_assistant: true,
                visible_phase: true,
                complete: row.complete,
                interrupted: false,
            }) {
                return false;
            }
        }
        true
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.partial_text.clear();
        self.input_active = false;
    }

    /// Fence queued and current speech. This has no task-control side effect.
    /// Already observed messages are blocked so their later streamed suffixes
    /// cannot restart the response the user just interrupted.
    pub fn cancel_speech(&mut self) -> u64 {
        let cancelled = self
            .current
            .as_ref()
            .map_or(self.generation, |s| s.generation);
        self.generation = self.generation.saturating_add(1);
        self.queue.clear();
        self.queued_chars = 0;
        self.current = None;
        self.paused_at = None;
        self.held.clear();
        for cursor in self.messages.values_mut() {
            cursor.blocked = true;
        }
        cancelled
    }

    pub fn pause(&mut self, generation: u64, now_ms: u64) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|s| s.generation == generation)
        {
            self.paused_at.get_or_insert(now_ms);
            true
        } else {
            false
        }
    }

    pub fn input_activity(&mut self, active: bool, now_ms: u64) {
        if !self.active || self.muted {
            return;
        }
        if self.input_active != active {
            self.input_changed_at = now_ms;
            self.input_active = active;
        }
    }

    pub fn resume(&mut self) -> Option<u64> {
        self.paused_at.take()?;
        self.current.as_ref().map(|s| s.generation)
    }

    pub fn resume_after_false_cue(&mut self, now_ms: u64) -> Option<u64> {
        let paused_at = self.paused_at?;
        if !self.input_active
            && self.partial_text.is_empty()
            && now_ms.saturating_sub(paused_at.max(self.input_changed_at)) >= FALSE_CUE_HOLD_MS
        {
            self.resume()
        } else {
            None
        }
    }

    pub fn transcript(
        &mut self,
        utterance_id: &str,
        revision: u64,
        text: &str,
        is_final: bool,
        requires_explicit_control: bool,
    ) -> InputDisposition {
        if !self.active || self.muted || utterance_id.is_empty() {
            return InputDisposition::Ignored;
        }
        if !self.utterances.contains_key(utterance_id) && self.utterances.len() >= MAX_UTTERANCES {
            return InputDisposition::Review;
        }
        let cursor = self.utterances.entry(utterance_id.to_owned()).or_default();
        if cursor.final_seen || revision < cursor.revision {
            return InputDisposition::Duplicate;
        }
        cursor.revision = revision;
        if text.chars().count() > MAX_TRANSCRIPT_CHARS {
            cursor.final_seen = is_final;
            return InputDisposition::Review;
        }
        let text = text.trim();
        if !is_final {
            self.partial_text = text.to_owned();
            return InputDisposition::Partial;
        }
        cursor.final_seen = true;
        self.partial_text.clear();
        self.input_active = false;
        if text.is_empty() {
            return InputDisposition::Ignored;
        }
        self.last_final_text = text.to_owned();
        if requires_explicit_control {
            return InputDisposition::Review;
        }
        let normalized = normalize_words(text);
        if self.current.is_some()
            && matches!(normalized.as_str(), "stop" | "stop speaking" | "be quiet")
        {
            return InputDisposition::StopSpeaking;
        }
        if self.current.is_some() && is_backchannel(&normalized) {
            return InputDisposition::Backchannel;
        }
        InputDisposition::Submit
    }

    /// Returns false when a bounded session has exhausted its transcript budget;
    /// the controller must end visibly instead of silently dropping new speech.
    pub fn observe(&mut self, message: AssistantText<'_>) -> bool {
        if !self.active
            || self.baseline.contains(message.id)
            || message.created_at < self.started_at
            || !message.direct_assistant
            || !message.visible_phase
        {
            return true;
        }
        if message.interrupted {
            self.held.remove(message.id);
            if self.messages.contains_key(message.id) {
                self.cancel_speech();
            }
            return true;
        }
        if !self.messages.contains_key(message.id) && self.messages.len() >= MAX_MESSAGES {
            return false;
        }
        if message.text.len() > MAX_SPEECH_CHARS * 4 {
            return false;
        }
        if self.waiting_for_admission {
            let replacing = self.held.get(message.id);
            let held_bytes = self.held.values().map(|m| m.text.len()).sum::<usize>();
            let replaced_bytes = replacing.map_or(0, |held| held.text.len());
            let projected_bytes = held_bytes
                .saturating_sub(replaced_bytes)
                .saturating_add(message.text.len());
            if (replacing.is_none() && self.held.len() >= MAX_MESSAGES)
                || projected_bytes > MAX_SPEECH_CHARS * 4
            {
                return false;
            }
            self.held.insert(
                message.id.to_owned(),
                HeldMessage {
                    text: message.text.to_owned(),
                    created_at: message.created_at,
                    complete: message.complete,
                },
            );
            return true;
        }
        let cursor = self.messages.entry(message.id.to_owned()).or_default();
        if cursor.blocked {
            return true;
        }
        if !message.text.starts_with(&cursor.text) {
            // Provider rewrote text that may already be audible. Never guess
            // which new suffix is safe, nor replay the whole replacement.
            cursor.blocked = true;
            self.cancel_speech();
            return true;
        }
        cursor.text = message.text.to_owned();
        let remainder = &message.text[cursor.consumed..];
        let boundary = stable_boundary(remainder, message.complete);
        if boundary == 0 {
            return true;
        }
        let speakable = speech_text(&remainder[..boundary]);
        cursor.consumed += boundary;
        for text in split_segments(&speakable, MAX_SEGMENT_CHARS) {
            let count = text.chars().count();
            if self.queued_chars.saturating_add(count) > MAX_SPEECH_CHARS {
                return false;
            }
            self.queued_chars += count;
            self.queue.push_back(SpeechSegment {
                message_id: message.id.to_owned(),
                generation: 0,
                text,
            });
        }
        true
    }

    pub fn next_speech(&mut self) -> Option<SpeechSegment> {
        if !self.active
            || self.waiting_for_admission
            || self.current.is_some()
            || self.input_active
            || !self.partial_text.is_empty()
        {
            return None;
        }
        let mut segment = self.queue.pop_front()?;
        self.queued_chars = self
            .queued_chars
            .saturating_sub(segment.text.chars().count());
        self.generation = self.generation.saturating_add(1);
        segment.generation = self.generation;
        self.current = Some(segment.clone());
        Some(segment)
    }

    pub fn completed(&mut self, generation: u64) -> bool {
        if !self
            .current
            .as_ref()
            .is_some_and(|s| s.generation == generation)
        {
            return false;
        }
        self.current = None;
        self.paused_at = None;
        true
    }
}

fn normalize_words(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_backchannel(text: &str) -> bool {
    // Deliberately omit yes/no/okay: those may be answers to a real question.
    matches!(text, "mm hmm" | "mhm" | "mmhmm" | "uh huh" | "aha")
}

fn stable_boundary(text: &str, complete: bool) -> usize {
    if complete {
        return text.len();
    }
    // Never split an unfinished fenced block, inline code or Markdown link.
    let mut fence: Option<(char, usize)> = None;
    let mut inline = false;
    let mut brackets: u32 = 0;
    let mut boundary = 0;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if update_fence(line, &mut fence) {
            offset += line.len();
            if fence.is_none() && line.ends_with('\n') {
                boundary = offset;
            }
            continue;
        }
        if fence.is_none() {
            for (index, ch) in line.char_indices() {
                match ch {
                    '`' => inline = !inline,
                    '[' | '(' if !inline => brackets = brackets.saturating_add(1),
                    ']' | ')' if !inline => brackets = brackets.saturating_sub(1),
                    _ => {}
                }
                let end = offset + index + ch.len_utf8();
                if !inline
                    && brackets == 0
                    && matches!(ch, '.' | '!' | '?' | '\n')
                    && text[end..].chars().next().is_some_and(char::is_whitespace)
                {
                    boundary = end;
                }
            }
        }
        offset += line.len();
    }
    boundary
}

/// Conservative speech projection: code, URLs and markup stay in the visual
/// transcript. It is a projection only; the canonical message is never changed.
pub fn speech_text(markdown: &str) -> String {
    let mut result = String::new();
    let mut fence = None;
    for line in markdown.lines() {
        let line = line.trim();
        if update_fence(line, &mut fence) {
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let mut inline = false;
        let mut link_destination = false;
        let mut previous = '\0';
        let mut projected = String::new();
        for ch in strip_line_marker(line).chars() {
            if ch == '`' {
                inline = !inline;
                continue;
            }
            if inline {
                continue;
            }
            if ch == '(' && previous == ']' {
                link_destination = true;
                previous = ch;
                continue;
            }
            if link_destination {
                if ch == ')' {
                    link_destination = false;
                }
                previous = ch;
                continue;
            }
            previous = ch;
            if matches!(ch, '[' | ']' | '*' | '_' | '<' | '>') {
                continue;
            }
            if !ch.is_control() {
                projected.push(ch);
            }
        }
        for word in projected.split_whitespace() {
            if word.contains("://")
                || word.starts_with("www.")
                || word.starts_with(":codex-file-citation")
            {
                continue;
            }
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(word);
        }
    }
    result
}

fn update_fence(line: &str, fence: &mut Option<(char, usize)>) -> bool {
    let line = line.trim_start();
    let Some(marker) = line.chars().next().filter(|ch| matches!(ch, '`' | '~')) else {
        return false;
    };
    let count = line.chars().take_while(|ch| *ch == marker).count();
    if count < 3 {
        return false;
    }
    match *fence {
        None => *fence = Some((marker, count)),
        Some((opening, width))
            if opening == marker && count >= width && line[count..].trim().is_empty() =>
        {
            *fence = None
        }
        _ => {}
    }
    true
}

fn strip_line_marker(line: &str) -> &str {
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("> "))
    {
        return rest.trim_start();
    }
    let heading = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&heading) && line[heading..].starts_with(char::is_whitespace) {
        return line[heading..].trim_start();
    }
    line
}

fn split_segments(text: &str, max_chars: usize) -> Vec<String> {
    let mut segments = Vec::new();
    let mut segment = String::new();
    let mut count = 0;
    for word in text.split_whitespace() {
        let size = word.chars().count();
        if count > 0 && count + 1 + size > max_chars {
            segments.push(std::mem::take(&mut segment));
            count = 0;
        }
        if size > max_chars {
            // Unbroken identifiers/encoded blobs are not useful spoken prose.
            continue;
        }
        if count > 0 {
            segment.push(' ');
            count += 1;
        }
        segment.push_str(word);
        count += size;
    }
    if !segment.is_empty() {
        segments.push(segment);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message<'a>(id: &'a str, text: &'a str, complete: bool) -> AssistantText<'a> {
        AssistantText {
            id,
            text,
            created_at: 100,
            direct_assistant: true,
            visible_phase: true,
            complete,
            interrupted: false,
        }
    }
    fn model() -> ConversationModel {
        let mut model = ConversationModel::default();
        model.begin(100, []);
        model
    }
    #[test]
    fn history_and_hidden_or_peer_text_never_speak() {
        let mut m = model();
        m.begin(100, ["old".into()]);
        m.observe(message("old", "Existing history.", true));
        let mut hidden = message("hidden", "Private reasoning.", true);
        hidden.visible_phase = false;
        m.observe(hidden);
        let mut peer = message("peer", "Peer context.", true);
        peer.direct_assistant = false;
        m.observe(peer);
        assert!(m.next_speech().is_none());
    }
    #[test]
    fn baseline_keeps_only_the_newest_bounded_history_ids() {
        let mut m = ConversationModel::default();
        m.begin(100, (0..10_000).map(|i| format!("id-{i}")));
        assert_eq!(m.baseline.len(), 512);
        assert!(!m.baseline.contains("id-9487"));
        assert!(m.baseline.contains("id-9488"));
        assert!(m.baseline.contains("id-9999"));
        assert!(m.observe(message("id-9487", "Outside the retained baseline.", true)));
        assert!(m.observe(message("id-9488", "Inside the retained baseline.", true)));
        assert_eq!(m.next_speech().unwrap().message_id, "id-9487");
    }
    #[test]
    fn only_completed_clauses_stream_once() {
        let mut m = model();
        m.observe(message("a", "One sentence. An unfinished", false));
        let first = m.next_speech().unwrap();
        assert_eq!(first.text, "One sentence.");
        m.observe(message("a", "One sentence. An unfinished", false));
        m.completed(first.generation);
        assert!(m.next_speech().is_none());
        m.observe(message("a", "One sentence. An unfinished thought.", true));
        assert_eq!(m.next_speech().unwrap().text, "An unfinished thought.");
    }
    #[test]
    fn cancelled_generation_and_late_suffix_cannot_resume() {
        let mut m = model();
        m.observe(message("a", "First. Second", false));
        let first = m.next_speech().unwrap();
        m.cancel_speech();
        assert!(!m.completed(first.generation));
        m.observe(message("a", "First. Second sentence.", true));
        assert!(m.next_speech().is_none());
        m.observe(message("b", "New answer.", true));
        assert!(m.next_speech().unwrap().generation > first.generation);
    }
    #[test]
    fn partial_revision_and_duplicate_final_never_submit_twice() {
        let mut m = model();
        assert_eq!(
            m.transcript("u", 1, "Tuesday", false, false),
            InputDisposition::Partial
        );
        assert_eq!(
            m.transcript("u", 2, "Wednesday", true, false),
            InputDisposition::Submit
        );
        assert_eq!(
            m.transcript("u", 3, "Thursday", true, false),
            InputDisposition::Duplicate
        );
        assert_eq!(m.last_final_text, "Wednesday");
    }
    #[test]
    fn backchannel_requires_playback_and_does_not_swallow_yes_or_correction() {
        let mut m = model();
        assert_eq!(
            m.transcript("idle", 1, "mm-hmm", true, false),
            InputDisposition::Submit
        );
        m.observe(message("a", "I am speaking.", true));
        m.next_speech();
        assert_eq!(
            m.transcript("b", 1, "mm-hmm", true, false),
            InputDisposition::Backchannel
        );
        assert_eq!(
            m.transcript("c", 1, "mm-hmm, but change the date", true, false),
            InputDisposition::Submit
        );
        assert_eq!(
            m.transcript("d", 1, "Yes", true, false),
            InputDisposition::Submit
        );
        assert_eq!(
            m.transcript("e", 1, "Yes", true, true),
            InputDisposition::Review
        );
    }
    #[test]
    fn stop_is_playback_control_only_during_playback() {
        let mut m = model();
        assert_eq!(
            m.transcript("a", 1, "stop", true, false),
            InputDisposition::Submit
        );
        m.observe(message("reply", "Speaking.", true));
        m.next_speech();
        assert_eq!(
            m.transcript("b", 1, "stop", true, false),
            InputDisposition::StopSpeaking
        );
        assert_eq!(
            m.transcript("c", 1, "stop the build", true, false),
            InputDisposition::Submit
        );
    }
    #[test]
    fn cough_resumes_but_partial_speech_holds() {
        let mut m = model();
        m.observe(message("a", "A reply.", true));
        let segment = m.next_speech().unwrap();
        m.pause(segment.generation, 100);
        m.input_activity(true, 100);
        m.input_activity(false, 200);
        assert_eq!(m.resume_after_false_cue(1999), None);
        assert_eq!(m.resume_after_false_cue(2000), Some(segment.generation));
        m.pause(segment.generation, 2100);
        m.transcript("p", 1, "Wait", false, false);
        assert_eq!(m.resume_after_false_cue(8000), None);
    }
    #[test]
    fn task_switch_and_mute_fence_input() {
        let mut m = model();
        m.set_muted(true);
        assert_eq!(
            m.transcript("a", 1, "Send", true, false),
            InputDisposition::Ignored
        );
        m.end();
        assert_eq!(
            m.transcript("a", 1, "Send", true, false),
            InputDisposition::Ignored
        );
    }
    #[test]
    fn rewrites_stop_already_queued_speech() {
        let mut m = model();
        m.observe(message("a", "The answer is one. More", false));
        m.next_speech();
        m.observe(message("a", "The answer is two.", true));
        assert!(!m.speaking());
        assert!(m.next_speech().is_none());
    }
    #[test]
    fn interrupted_rows_stop_audio_and_admission_holds_new_old_reply_ids() {
        let mut m = model();
        m.observe(message("a", "Speaking.", true));
        m.next_speech();
        let mut row = message("a", "Speaking.", true);
        row.interrupted = true;
        m.observe(row);
        assert!(!m.speaking());
        m.await_admission();
        m.observe(message("late", "Older reply arriving late.", true));
        m.admission_delivered(101);
        assert!(m.next_speech().is_none());
        m.observe(message("fresh", "New response.", true));
        assert_eq!(m.next_speech().unwrap().message_id, "fresh");
    }
    #[test]
    fn valid_fast_response_waits_for_receipt_without_getting_lost() {
        let mut m = model();
        m.await_admission();
        m.observe(message("fresh", "A fast response.", true));
        assert!(m.next_speech().is_none());
        assert!(m.admission_delivered(100));
        assert_eq!(m.next_speech().unwrap().text, "A fast response.");
    }
    #[test]
    fn held_stream_replacements_count_only_the_latest_body() {
        let mut m = model();
        m.await_admission();
        let first = "a".repeat(60_000);
        let replacement = "b".repeat(90_000);
        assert!(m.observe(message("stream", &first, false)));
        assert!(m.observe(message("stream", &replacement, false)));
        assert_eq!(m.held.len(), 1);
        assert_eq!(m.held["stream"].text, replacement);
    }
    #[test]
    fn held_message_at_capacity_can_receive_stream_updates() {
        let mut m = model();
        m.await_admission();
        for index in 0..MAX_MESSAGES {
            assert!(m.observe(message(&format!("stream-{index}"), "Growing.", false)));
        }
        assert_eq!(m.held.len(), MAX_MESSAGES);
        assert!(m.observe(message("stream-0", "Growing safely.", false)));
        assert_eq!(m.held["stream-0"].text, "Growing safely.");
        assert_eq!(m.held.len(), MAX_MESSAGES);
    }
    #[test]
    fn mismatched_fences_and_literal_hyphens_are_preserved_safely() {
        assert_eq!(
            speech_text(
                "Before\n````rust\nsecret\n```\nstill secret\n~~~\nalso secret\n````\nAfter"
            ),
            "Before After"
        );
        assert_eq!(
            speech_text("-3 degrees\n--no-verify\n- list item\n# Heading"),
            "-3 degrees --no-verify list item Heading"
        );
    }
    #[test]
    fn projection_omits_code_and_destinations() {
        assert_eq!(
            speech_text(
                "## Hello **there**\n```rust\nsecret();\n```\nRead [this](https://example.com) and `code` later."
            ),
            "Hello there Read this and later."
        );
        assert_eq!(stable_boundary("Before. ```unfinished", false), 7);
        assert_eq!(stable_boundary("[link](https://example.com", false), 0);
    }
    #[test]
    fn segmentation_preserves_unicode_and_bounds() {
        let text = "été 🙂 ".repeat(200);
        let segments = split_segments(&text, 32);
        assert!(segments.iter().all(|s| s.chars().count() <= 32));
        assert_eq!(segments.join(" "), text.trim());
    }
}
