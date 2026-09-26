//! Bounded live projections and a revision journal. Full transcripts remain in
//! durable history and are read explicitly by stable message-ID cursors.

use crate::history::{History, HistoryRecord};
use crate::model::{ChannelMessage, Message, Snapshot};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const LIVE_MESSAGES_PER_TASK: usize = 64;
pub const MAX_MESSAGE_PAGE: usize = 100;
const MAX_JOURNAL_REVISIONS: usize = 128;
const MAX_JOURNAL_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDeltaResponse {
    pub revision: String,
    pub snapshot: Option<Snapshot>,
    pub delta: Option<UiDelta>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDelta {
    pub from_revision: String,
    /// Current metadata and compact activity. Its messages array is empty;
    /// clients keep their loaded pages and apply the explicit message changes.
    pub metadata: Snapshot,
    pub messages: Vec<Message>,
    pub removed_message_ids: Vec<String>,
    pub retained_channel_ids: Vec<String>,
    pub channel_message_changes: Vec<ChannelMessageChange>,
    pub retained_subagent_transcript_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMessagesPage {
    /// Chronological within this page.
    pub messages: Vec<Message>,
    pub next_before_id: Option<String>,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessageChange {
    pub channel_id: String,
    pub messages: Vec<ChannelMessage>,
    pub removed_message_ids: Vec<String>,
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessagesPage {
    pub channel_id: String,
    pub messages: Vec<ChannelMessage>,
    pub next_before_id: Option<String>,
    pub revision: String,
}

struct Revision {
    number: u64,
    changes: Option<(Vec<Message>, Vec<String>)>,
    bytes: usize,
    changed_channels: Option<BTreeSet<String>>,
    channel_changes: Option<Vec<ChannelMessageChange>>,
    changed_subagents: Option<BTreeSet<String>>,
}

#[derive(Default)]
pub struct Journal {
    revisions: VecDeque<Revision>,
    bytes: usize,
}

impl Journal {
    /// None means a structural history edit requires a fresh bounded snapshot.
    /// A failed durable transaction never reaches this method.
    pub fn record(&mut self, number: u64, changes: Option<(Vec<Message>, Vec<String>)>) {
        let bytes = changes.as_ref().map_or(0, |(messages, removed)| {
            // Attachment metadata is part of a message; use its actual wire
            // size to cap retained payloads, including data URLs.
            serde_json::to_vec(messages).map_or(MAX_JOURNAL_BYTES + 1, |data| data.len())
                + removed.iter().map(String::len).sum::<usize>()
        });
        let (changes, bytes) = if bytes > MAX_JOURNAL_BYTES {
            (None, 0)
        } else {
            (changes, bytes)
        };
        self.bytes += bytes;
        self.revisions.push_back(Revision {
            number,
            changes,
            bytes,
            changed_channels: None,
            channel_changes: None,
            changed_subagents: None,
        });
        while self.revisions.len() > MAX_JOURNAL_REVISIONS || self.bytes > MAX_JOURNAL_BYTES {
            if let Some(expired) = self.revisions.pop_front() {
                self.bytes -= expired.bytes;
            }
        }
    }

    pub fn record_snapshot(&mut self, number: u64, before: &Snapshot, after: &Snapshot) {
        self.record(number, message_changes(before, after));
        let channel_changes: Vec<_> = after
            .channels
            .iter()
            .filter_map(|channel| {
                let old = before.channels.iter().find(|old| old.id == channel.id);
                if old.is_some_and(|old| old.messages == channel.messages) {
                    return None;
                }
                let changes = old.and_then(|old| history_changes(&old.messages, &channel.messages));
                let reset = changes.is_none();
                let (messages, removed_message_ids) =
                    changes.unwrap_or_else(|| (channel_tail(&channel.messages), vec![]));
                Some(ChannelMessageChange {
                    channel_id: channel.id.clone(),
                    messages,
                    removed_message_ids,
                    reset,
                })
            })
            .collect();
        if let Some(revision) = self.revisions.back_mut() {
            let extra = serde_json::to_vec(&channel_changes)
                .map_or(MAX_JOURNAL_BYTES + 1, |value| value.len());
            if revision.bytes.saturating_add(extra) > MAX_JOURNAL_BYTES {
                self.bytes -= revision.bytes;
                revision.bytes = 0;
                revision.changes = None;
            } else {
                revision.bytes += extra;
                self.bytes += extra;
                revision.channel_changes = Some(channel_changes);
            }
            revision.changed_channels = Some(
                after
                    .channels
                    .iter()
                    .filter(|channel| {
                        !before
                            .channels
                            .iter()
                            .any(|old| old.id == channel.id && old.messages == channel.messages)
                    })
                    .map(|channel| channel.id.clone())
                    .collect(),
            );
            revision.changed_subagents = Some(
                after
                    .subagent_transcripts
                    .iter()
                    .filter(|(id, entries)| before.subagent_transcripts.get(*id) != Some(*entries))
                    .map(|(id, _)| id.clone())
                    .collect(),
            );
        }
        while self.bytes > MAX_JOURNAL_BYTES {
            if let Some(expired) = self.revisions.pop_front() {
                self.bytes -= expired.bytes;
            } else {
                break;
            }
        }
    }

    fn channel_changes(
        &self,
        from: u64,
        through: u64,
        snapshot: &Snapshot,
    ) -> Option<Vec<ChannelMessageChange>> {
        let mut combined: BTreeMap<String, ChannelMessageChange> = BTreeMap::new();
        for revision in self
            .revisions
            .iter()
            .filter(|revision| revision.number > from && revision.number <= through)
        {
            for change in revision.channel_changes.as_ref()? {
                let current = combined
                    .entry(change.channel_id.clone())
                    .or_insert_with(|| ChannelMessageChange {
                        channel_id: change.channel_id.clone(),
                        messages: vec![],
                        removed_message_ids: vec![],
                        reset: false,
                    });
                if change.reset {
                    current.reset = true;
                    current.messages.clear();
                    current.removed_message_ids.clear();
                }
                if current.reset {
                    continue;
                }
                let mut messages: BTreeMap<_, _> = current
                    .messages
                    .drain(..)
                    .map(|message| (message.id.clone(), message))
                    .collect();
                let mut removed: BTreeSet<_> = current.removed_message_ids.drain(..).collect();
                for id in &change.removed_message_ids {
                    messages.remove(id);
                    removed.insert(id.clone());
                }
                for message in &change.messages {
                    removed.remove(&message.id);
                    messages.insert(message.id.clone(), message.clone());
                }
                current.messages = messages.into_values().collect();
                current.removed_message_ids = removed.into_iter().collect();
            }
        }
        let mut result = Vec::new();
        for (_, mut change) in combined {
            // Deletion is represented by missing metadata, including delete+recreate histories.
            let Some(channel) = snapshot
                .channels
                .iter()
                .find(|channel| channel.id == change.channel_id)
            else {
                continue;
            };
            if change.reset {
                change.messages = channel_tail(&channel.messages);
            } else {
                change
                    .messages
                    .sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
            }
            result.push(change);
        }
        Some(result)
    }

    fn retained(&self, from: u64, through: u64, snapshot: &Snapshot) -> (Vec<String>, Vec<String>) {
        let revisions = self
            .revisions
            .iter()
            .filter(|item| item.number > from && item.number <= through)
            .collect::<Vec<_>>();
        if revisions
            .iter()
            .any(|item| item.changed_channels.is_none() || item.changed_subagents.is_none())
        {
            return (vec![], vec![]);
        }
        let channels = snapshot
            .channels
            .iter()
            .filter(|channel| {
                !revisions.iter().any(|item| {
                    item.changed_channels
                        .as_ref()
                        .unwrap()
                        .contains(&channel.id)
                })
            })
            .map(|channel| channel.id.clone())
            .collect();
        let subagents = snapshot
            .subagent_transcripts
            .keys()
            .filter(|id| {
                !revisions
                    .iter()
                    .any(|item| item.changed_subagents.as_ref().unwrap().contains(*id))
            })
            .cloned()
            .collect();
        (channels, subagents)
    }

    pub fn changes(&self, from: u64, through: u64) -> Option<(Vec<Message>, Vec<String>)> {
        if from > through {
            return None;
        }
        if from == through {
            return Some((vec![], vec![]));
        }
        let mut expected = from.checked_add(1)?;
        let mut messages = BTreeMap::new();
        let mut removed = BTreeSet::new();
        for revision in self
            .revisions
            .iter()
            .filter(|item| item.number > from && item.number <= through)
        {
            if revision.number != expected {
                return None;
            }
            let (upserts, deletions) = revision.changes.as_ref()?;
            for id in deletions {
                messages.remove(id);
                removed.insert(id.clone());
            }
            for message in upserts {
                removed.remove(&message.id);
                messages.insert(message.id.clone(), message.clone());
            }
            expected = expected.checked_add(1)?;
        }
        if expected != through.checked_add(1)? {
            return None;
        }
        let mut messages = messages.into_values().collect::<Vec<_>>();
        messages
            .sort_by(|left, right| (left.created_at, &left.id).cmp(&(right.created_at, &right.id)));
        Some((messages, removed.into_iter().collect()))
    }
}

pub fn parse_revision(revision: &str, epoch: &str) -> Option<u64> {
    let (source, number) = revision.rsplit_once(':')?;
    (source == epoch)
        .then(|| number.parse::<u64>().ok())
        .flatten()
}

pub fn message_changes(before: &Snapshot, after: &Snapshot) -> Option<(Vec<Message>, Vec<String>)> {
    history_changes(&before.messages, &after.messages)
}

fn history_changes<T: HistoryRecord>(
    before: &History<T>,
    after: &History<T>,
) -> Option<(Vec<T>, Vec<String>)> {
    use crate::history::{HistoryChange, HistoryDelta};
    let HistoryDelta::Incremental { changes, .. } = after.delta_since(&before) else {
        return None;
    };
    if changes.len() > 512 {
        return None;
    }
    let mut touched = BTreeSet::new();
    let mut removed = BTreeSet::new();
    for change in changes {
        match change {
            HistoryChange::Append { index } | HistoryChange::Update { index } => {
                touched.insert(index);
            }
            HistoryChange::TruncateFrom { new_len } => {
                if before.len().saturating_sub(new_len) > 512 {
                    return None;
                }
                for index in new_len..before.len() {
                    removed.insert(before[index].id().to_owned());
                }
            }
        }
    }
    let mut messages = Vec::new();
    for index in touched {
        if let Some(message) = after.get(index) {
            if let Some(old) = before.get(index) {
                if old.id() != message.id() {
                    removed.insert(old.id().to_owned());
                }
            }
            removed.remove(message.id());
            messages.push(message.clone());
        }
    }
    Some((messages, removed.into_iter().collect()))
}

fn channel_tail(messages: &History<ChannelMessage>) -> Vec<ChannelMessage> {
    let start = messages.len().saturating_sub(LIVE_MESSAGES_PER_TASK);
    (start..messages.len())
        .map(|index| messages[index].clone())
        .collect()
}

/// The work here is bounded per task, regardless of the amount of retained
/// history. Each scope index is persistent and only its newest rows are read.
pub fn projection(snapshot: &Snapshot, include_messages: bool) -> Snapshot {
    let mut output = snapshot.clone();
    let mut messages = Vec::new();
    for task in &snapshot.tasks {
        if include_messages {
            let positions = snapshot.messages.scope_indices(&task.id);
            messages.extend(positions.iter().rev().take(LIVE_MESSAGES_PER_TASK).copied());
        }
    }
    messages.sort_unstable();
    output.messages = messages
        .into_iter()
        .map(|index| snapshot.messages[index].clone())
        .collect();
    for channel in &mut output.channels {
        channel.messages = channel_tail(&channel.messages).into();
    }
    output.events = compact_events(snapshot);
    output
}

pub fn compact_events(
    snapshot: &Snapshot,
) -> crate::history::History<std::sync::Arc<crate::model::RunEvent>> {
    let mut events = Vec::new();
    for task_id in snapshot.events.scope_ids() {
        let positions = snapshot.events.scope_indices(&task_id);
        events.extend(
            positions
                .iter()
                .rev()
                .take(512)
                .filter(|index| !matches!(snapshot.events[**index].kind.as_str(), "log" | "output"))
                .take(crate::lan_sync::MAX_LIVE_EVENTS_PER_TASK)
                .copied(),
        );
    }
    events.sort_unstable();
    let start = events
        .len()
        .saturating_sub(crate::lan_sync::MAX_LIVE_EVENTS);
    events[start..]
        .iter()
        .map(|index| std::sync::Arc::new(crate::lan_sync::compact_event(&snapshot.events[*index])))
        .collect()
}

pub fn message_page(
    snapshot: &Snapshot,
    task_id: &str,
    before_id: Option<&str>,
    limit: Option<u32>,
    revision: String,
) -> Result<TaskMessagesPage, String> {
    if !snapshot.tasks.iter().any(|task| task.id == task_id) {
        return Err("Task was not found.".into());
    }
    let positions = snapshot.messages.scope_indices(task_id);
    let before = match before_id {
        Some(id) => {
            let index = snapshot
                .messages
                .index_of_id(id)
                .map_err(|error| error.to_string())?
                .ok_or(
                    "This transcript cursor is no longer available. Refresh the conversation.",
                )?;
            if snapshot.messages[index].task_id != task_id {
                return Err("Transcript cursor belongs to another task.".into());
            }
            index
        }
        None => snapshot.messages.len(),
    };
    // Binary search the ordered scope index so a deep page never walks all
    // newer rows. Global append positions preserve cursor identity.
    let mut low = 0;
    let mut high = positions.len();
    while low < high {
        let mid = low + (high - low) / 2;
        if positions[mid] < before {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let end = low;
    let start = end.saturating_sub(limit.unwrap_or(64).clamp(1, MAX_MESSAGE_PAGE as u32) as usize);
    let messages: Vec<_> = (start..end)
        .map(|index| snapshot.messages[positions[index]].clone())
        .collect();
    let next_before_id = if start > 0 {
        messages.first().map(|message| message.id.clone())
    } else {
        None
    };
    Ok(TaskMessagesPage {
        messages,
        next_before_id,
        revision,
    })
}

pub fn channel_message_page(
    snapshot: &Snapshot,
    channel_id: &str,
    before_id: Option<&str>,
    limit: Option<u32>,
    revision: String,
) -> Result<ChannelMessagesPage, String> {
    let channel = snapshot
        .channels
        .iter()
        .find(|channel| channel.id == channel_id)
        .ok_or("Channel was not found.")?;
    let end = match before_id {
        Some(id) => channel
            .messages
            .index_of_id(id)
            .map_err(|error| error.to_string())?
            .ok_or("This channel cursor is no longer available. Refresh the conversation.")?,
        None => channel.messages.len(),
    };
    let start = end.saturating_sub(
        limit
            .unwrap_or(LIVE_MESSAGES_PER_TASK as u32)
            .clamp(1, MAX_MESSAGE_PAGE as u32) as usize,
    );
    let messages: Vec<_> = (start..end)
        .map(|index| channel.messages[index].clone())
        .collect();
    let next_before_id = (start > 0).then(|| channel.messages[start].id.clone());
    Ok(ChannelMessagesPage {
        channel_id: channel_id.into(),
        messages,
        next_before_id,
        revision,
    })
}

impl crate::Service {
    pub(crate) fn channel_messages(
        &self,
        channel_id: &str,
        before_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<ChannelMessagesPage, String> {
        let data = self.committed_data()?;
        channel_message_page(
            &data.snapshot,
            channel_id,
            before_id,
            limit,
            format!("{}:{}", self.revision_epoch, data.revision),
        )
    }

    pub(crate) fn ui_delta(&self, revision: Option<&str>) -> Result<UiDeltaResponse, String> {
        let data = self.committed_data()?;
        let current = format!("{}:{}", self.revision_epoch, data.revision);
        if revision == Some(current.as_str()) {
            return Ok(UiDeltaResponse {
                revision: current,
                snapshot: None,
                delta: None,
            });
        }
        let changes = revision
            .and_then(|value| parse_revision(value, &self.revision_epoch))
            .and_then(|from| {
                let journal = self.ui_journal.lock().ok()?;
                let changes = journal.changes(from, data.revision)?;
                let retained = journal.retained(from, data.revision, &data.snapshot);
                let channels = journal.channel_changes(from, data.revision, &data.snapshot)?;
                Some((changes, retained, channels))
            });
        if let Some((
            (messages, removed_message_ids),
            (retained_channel_ids, retained_subagent_transcript_ids),
            channel_message_changes,
        )) = changes
        {
            let mut metadata = projection(&data.snapshot, false);
            for channel in &mut metadata.channels {
                channel.messages.clear();
            }
            for id in &retained_subagent_transcript_ids {
                metadata.subagent_transcripts.remove(id);
            }
            Ok(UiDeltaResponse {
                revision: current,
                snapshot: None,
                delta: Some(UiDelta {
                    from_revision: revision.unwrap().into(),
                    metadata,
                    messages,
                    removed_message_ids,
                    retained_channel_ids,
                    channel_message_changes,
                    retained_subagent_transcript_ids,
                }),
            })
        } else {
            Ok(UiDeltaResponse {
                revision: current,
                snapshot: Some(projection(&data.snapshot, true)),
                delta: None,
            })
        }
    }

    pub(crate) fn task_messages(
        &self,
        task_id: &str,
        before_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<TaskMessagesPage, String> {
        let data = self.committed_data()?;
        message_page(
            &data.snapshot,
            task_id,
            before_id,
            limit,
            format!("{}:{}", self.revision_epoch, data.revision),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Channel, ChannelMessage};

    fn message(id: &str, text: &str) -> Message {
        Message {
            id: id.into(),
            task_id: "task".into(),
            role: "assistant".into(),
            text: text.into(),
            created_at: 1,
            sender_agent_id: None,
            collaboration_id: None,
            attachments: vec![],
            phase: None,
            response_metadata: None,
            stream_status: Some("streaming".into()),
        }
    }

    fn channel(id: &str, text: &str) -> Channel {
        Channel {
            id: id.into(),
            name: "fixture channel".into(),
            description: String::new(),
            agent_ids: vec![],
            messages: vec![ChannelMessage {
                id: "channel-message".into(),
                role: "assistant".into(),
                agent_id: None,
                text: text.into(),
                created_at: 1,
                task_id: None,
            }]
            .into(),
            agent_conversation_enabled: false,
            agent_conversation_turn_limit: 6,
            agent_conversation_turns_used: 0,
            agent_conversation_paused: false,
        }
    }

    #[test]
    fn channel_history_deltas_and_pages_are_bounded_and_preserve_identity() {
        let mut before = crate::model::default_snapshot();
        let mut large = channel("large", "seed");
        large.messages = (0..10_000)
            .map(|index| ChannelMessage {
                id: format!("channel-{index:05}"),
                text: "history".repeat(20),
                created_at: index,
                role: "assistant".into(),
                agent_id: None,
                task_id: None,
            })
            .collect();
        before.channels.push(large);
        let projected = projection(&before, true);
        assert_eq!(projected.channels[0].messages.len(), LIVE_MESSAGES_PER_TASK);
        assert_eq!(projected.channels[0].messages[0].id, "channel-09936");
        let page = channel_message_page(
            &before,
            "large",
            Some("channel-09936"),
            Some(500),
            "epoch:0".into(),
        )
        .unwrap();
        assert_eq!(page.messages.len(), MAX_MESSAGE_PAGE);
        assert_eq!(page.messages[0].id, "channel-09836");
        assert_eq!(page.next_before_id.as_deref(), Some("channel-09836"));
        assert!(
            channel_message_page(&before, "large", Some("missing"), None, "epoch:0".into())
                .is_err()
        );
        assert!(channel_message_page(&before, "missing", None, None, "epoch:0".into()).is_err());
        let mut after = before.clone();
        after.channels[0]
            .messages
            .get_mut_tracked(9_999)
            .unwrap()
            .text = "fresh".into();
        let mut journal = Journal::default();
        journal.record_snapshot(1, &before, &after);
        let changes = journal.channel_changes(0, 1, &after).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(!changes[0].reset);
        assert_eq!(changes[0].messages.len(), 1);
        assert_eq!(changes[0].messages[0].text, "fresh");
        assert!(serde_json::to_vec(&changes).unwrap().len() < 1024);
        let mut truncated = after.clone();
        truncated.channels[0].messages.truncate(9_999);
        journal.record_snapshot(2, &after, &truncated);
        let changes = journal.channel_changes(0, 2, &truncated).unwrap();
        assert!(changes[0].messages.is_empty());
        assert_eq!(changes[0].removed_message_ids, vec!["channel-09999"]);
        let mut replaced = truncated.clone();
        replaced.channels[0].messages = vec![ChannelMessage {
            id: "new".into(),
            ..before.channels[0].messages[0].clone()
        }]
        .into();
        journal.record_snapshot(3, &truncated, &replaced);
        let changes = journal.channel_changes(0, 3, &replaced).unwrap();
        assert!(changes[0].reset);
        assert_eq!(changes[0].messages.len(), 1);
        assert_eq!(changes[0].messages[0].id, "new");
        assert!(changes[0].removed_message_ids.is_empty());
        let mut oversized = replaced.clone();
        oversized.channels[0]
            .messages
            .get_mut_tracked(0)
            .unwrap()
            .text = "x".repeat(MAX_JOURNAL_BYTES + 1);
        journal.record_snapshot(4, &replaced, &oversized);
        assert!(journal.bytes <= MAX_JOURNAL_BYTES);
        assert!(journal.changes(3, 4).is_none());
    }

    #[test]
    fn retained_channel_journal_does_not_reuse_deleted_id_body() {
        let mut before = crate::model::default_snapshot();
        before.channels.push(channel("reused", "old channel body"));
        let mut deleted = before.clone();
        deleted.channels.clear();
        let mut recreated = deleted.clone();
        recreated
            .channels
            .push(channel("reused", "new channel body"));

        let mut journal = Journal::default();
        journal.record_snapshot(1, &before, &deleted);
        journal.record_snapshot(2, &deleted, &recreated);
        let (retained, _) = journal.retained(0, 2, &recreated);
        assert!(!retained.iter().any(|id| id == "reused"));
        let mut metadata = projection(&recreated, false);
        for entry in &mut metadata.channels {
            if retained.contains(&entry.id) {
                entry.messages.clear();
            }
        }
        assert_eq!(metadata.channels.len(), 1);
        assert_eq!(metadata.channels[0].messages[0].text, "new channel body");

        journal.record_snapshot(3, &recreated, &deleted);
        let (retained_after_delete, _) = journal.retained(2, 3, &deleted);
        assert!(retained_after_delete.is_empty());
        assert!(projection(&deleted, false).channels.is_empty());
    }

    #[test]
    fn journal_coalesces_updates_and_preserves_explicit_deletions() {
        let mut journal = Journal::default();
        journal.record(
            1,
            Some((vec![message("a", "one"), message("b", "removed")], vec![])),
        );
        journal.record(2, Some((vec![message("a", "two")], vec!["b".into()])));
        let (messages, removed) = journal.changes(0, 2).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "two");
        assert_eq!(removed, vec!["b"]);
        assert!(journal.changes(0, 3).is_none());
        assert_eq!(journal.changes(1, 1).unwrap(), (vec![], vec![]));
    }

    #[test]
    fn expired_structural_future_and_cross_process_revisions_reset() {
        let mut journal = Journal::default();
        journal.record(1, Some((vec![], vec![])));
        journal.record(2, None);
        assert!(journal.changes(1, 2).is_none());
        for number in 3..=150 {
            journal.record(number, Some((vec![], vec![])));
        }
        assert!(journal.changes(1, 150).is_none());
        assert!(journal.changes(149, 150).is_some());
        assert!(journal.changes(151, 150).is_none());
        assert_eq!(parse_revision("epoch:42", "epoch"), Some(42));
        assert_eq!(parse_revision("old:42", "epoch"), None);
        assert_eq!(parse_revision("epoch:invalid", "epoch"), None);
    }

    #[test]
    fn oversized_frames_do_not_make_the_journal_unbounded() {
        let mut journal = Journal::default();
        journal.record(
            1,
            Some((
                vec![message("a", &"x".repeat(MAX_JOURNAL_BYTES + 1))],
                vec![],
            )),
        );
        assert_eq!(journal.bytes, 0);
        assert!(journal.changes(0, 1).is_none());
        journal.record(2, Some((vec![], vec![])));
        assert!(journal.changes(1, 2).is_some());
    }

    #[test]
    fn service_pages_full_history_and_only_publishes_committed_message_deltas() {
        let directory =
            std::env::temp_dir().join(format!("monitter-ui-sync-{}", crate::model::id()));
        let service = crate::Service::open(None, directory.clone()).unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(crate::CreateTaskInput {
                agent_id,
                title: "Synthetic paging fixture".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        service
            .mutate(None, |state| {
                state.messages.clear();
                for index in 0..150 {
                    let mut row = message(&format!("row-{index}"), "history");
                    row.task_id = task.id.clone();
                    row.created_at = index;
                    state.messages.push(row);
                }
                Ok(())
            })
            .unwrap();
        let initial = service.ui_delta(None).unwrap();
        assert_eq!(
            initial.snapshot.as_ref().unwrap().messages.len(),
            LIVE_MESSAGES_PER_TASK
        );
        let first = service.task_messages(&task.id, None, Some(100)).unwrap();
        assert_eq!(first.messages.len(), 100);
        let cursor = first.next_before_id.unwrap();
        service
            .mutate(None, |state| {
                state
                    .messages
                    .update_by_id("row-149", |row| row.text = "stream update".into())
                    .unwrap();
                Ok(())
            })
            .unwrap();
        let delta = service.ui_delta(Some(&initial.revision)).unwrap();
        let change = delta.delta.as_ref().unwrap();
        assert!(delta.snapshot.is_none());
        assert!(change.metadata.messages.is_empty());
        assert_eq!(change.messages.len(), 1);
        assert_eq!(change.messages[0].text, "stream update");
        let older = service
            .task_messages(&task.id, Some(&cursor), Some(100))
            .unwrap();
        assert_eq!(older.messages.len(), 50);
        assert!(older.next_before_id.is_none());
        assert_eq!(older.messages.last().unwrap().id, "row-49");
        assert!(service
            .task_messages(&task.id, Some("missing"), None)
            .is_err());
        assert!(service
            .task_messages("other-task", Some("row-20"), None)
            .is_err());
        let before_failed_mutation = delta.revision;
        let failed: Result<(), String> = service.mutate(None, |state| {
            state
                .messages
                .update_by_id("row-149", |row| row.text = "uncommitted".into())
                .unwrap();
            Err("synthetic rollback".into())
        });
        assert!(failed.is_err());
        let unchanged = service.ui_delta(Some(&before_failed_mutation)).unwrap();
        assert_eq!(unchanged.revision, before_failed_mutation);
        assert!(unchanged.snapshot.is_none() && unchanged.delta.is_none());
        service
            .mutate(None, |state| {
                state
                    .messages
                    .retain(|row| row.id != "row-20" && row.id != cursor);
                Ok(())
            })
            .unwrap();
        let reset = service.ui_delta(Some(&before_failed_mutation)).unwrap();
        assert!(reset.snapshot.is_some());
        assert!(reset.delta.is_none());
        assert!(service
            .task_messages(&task.id, Some(&cursor), Some(100))
            .is_err());
        assert_eq!(service.snapshot().unwrap().messages.len(), 148);
        drop(service);
        let reopened = crate::Service::open(None, directory.clone()).unwrap();
        let restored = reopened.snapshot().unwrap();
        assert_eq!(restored.messages.len(), 148);
        assert_eq!(
            restored
                .messages
                .get_by_id("row-149")
                .unwrap()
                .unwrap()
                .text,
            "stream update"
        );
        assert!(reopened
            .ui_delta(Some(&before_failed_mutation))
            .unwrap()
            .snapshot
            .is_some());
        drop(reopened);
        let _ = std::fs::remove_dir_all(directory);
    }
}
