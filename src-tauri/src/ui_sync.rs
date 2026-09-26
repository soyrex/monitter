//! Bounded live projections and a revision journal. Full transcripts remain in
//! durable history and are read explicitly by stable message-ID cursors.

use crate::model::{Message, Snapshot};
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

struct Revision {
    number: u64,
    changes: Option<(Vec<Message>, Vec<String>)>,
    bytes: usize,
    changed_channels: Option<BTreeSet<String>>,
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
        if let Some(revision) = self.revisions.back_mut() {
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
    use crate::history::{HistoryChange, HistoryDelta};
    let HistoryDelta::Incremental { changes, .. } = after.messages.delta_since(&before.messages)
    else {
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
                if before.messages.len().saturating_sub(new_len) > 512 {
                    return None;
                }
                for index in new_len..before.messages.len() {
                    removed.insert(before.messages[index].id.clone());
                }
            }
        }
    }
    let mut messages = Vec::new();
    for index in touched {
        if let Some(message) = after.messages.get(index) {
            if let Some(old) = before.messages.get(index) {
                if old.id != message.id {
                    removed.insert(old.id.clone());
                }
            }
            removed.remove(&message.id);
            messages.push(message.clone());
        }
    }
    Some((messages, removed.into_iter().collect()))
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

impl crate::Service {
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
                Some((changes, retained))
            });
        if let Some((
            (messages, removed_message_ids),
            (retained_channel_ids, retained_subagent_transcript_ids),
        )) = changes
        {
            let mut metadata = projection(&data.snapshot, false);
            for channel in &mut metadata.channels {
                if retained_channel_ids.contains(&channel.id) {
                    channel.messages.clear();
                }
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
