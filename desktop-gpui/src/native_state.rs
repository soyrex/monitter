//! Typed retained native UI state. No transport, JSON conversion, or I/O.
//! Page cursors belong to the caller: FullReset invalidates all of them and
//! reset_channel_ids invalidates the named channel cursors. A page's next cursor
//! is usable only after its merge succeeds at the current revision.
use monitter_lib::{
    experimental_desktop::{ChannelMessagesPage, TaskMessagesPage, UiDeltaResponse},
    model::{ChannelMessage, Message, Snapshot},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    pub changed_message_ids: Vec<String>,
    pub removed_message_ids: Vec<String>,
    pub removed_task_ids: Vec<String>,
    pub changed_channel_ids: Vec<String>,
    pub reset_channel_ids: Vec<String>,
    pub dropped_channel_ids: Vec<String>,
    pub changed_subagent_transcript_ids: Vec<String>,
    pub dropped_subagent_transcript_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyResult {
    FullReset(ChangeSet),
    Delta(ChangeSet),
    Unchanged,
    Gap,
}
#[derive(Default)]
pub struct NativeState {
    snapshot: Option<Snapshot>,
    revision: Option<String>,
}
fn unique(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    ids.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn merge<T: Clone>(
    old: impl Iterator<Item = T>,
    updates: Vec<T>,
    removed: &[String],
    id: impl Fn(&T) -> &str,
    time: impl Fn(&T) -> i64,
) -> Vec<T> {
    let removed: BTreeSet<&str> = removed.iter().map(String::as_str).collect();
    let mut values: BTreeMap<String, T> = old
        .filter(|row| !removed.contains(id(row)))
        .map(|row| (id(&row).to_owned(), row))
        .collect();
    for row in updates {
        if !removed.contains(id(&row)) {
            values.insert(id(&row).to_owned(), row);
        }
    }
    let mut values: Vec<T> = values.into_values().collect();
    values.sort_by(|a, b| time(a).cmp(&time(b)).then_with(|| id(a).cmp(id(b))));
    values
}
fn merge_messages(
    old: impl Iterator<Item = Message>,
    updates: Vec<Message>,
    removed: &[String],
) -> Vec<Message> {
    merge(old, updates, removed, |row| &row.id, |row| row.created_at)
}
fn merge_channel(
    old: impl Iterator<Item = ChannelMessage>,
    updates: Vec<ChannelMessage>,
    removed: &[String],
) -> Vec<ChannelMessage> {
    merge(old, updates, removed, |row| &row.id, |row| row.created_at)
}
impl NativeState {
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }
    /// UI-only page eviction. SQLite remains authoritative and this does not
    /// change the journal revision or report authoritative message removals.
    pub fn trim_task_messages(&mut self, task_id: &str, keep: usize) -> bool {
        let Some(snapshot) = self.snapshot.as_mut() else { return false; };
        let count = snapshot.messages.iter().filter(|row| row.task_id == task_id).count();
        let mut drop = count.saturating_sub(keep);
        if drop == 0 { return false; }
        snapshot.messages.retain(|row| {
            if row.task_id == task_id && drop > 0 {
                drop -= 1;
                false
            } else { true }
        });
        true
    }
    pub fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }
    pub fn apply(&mut self, response: UiDeltaResponse) -> ApplyResult {
        let UiDeltaResponse {
            revision,
            snapshot,
            delta,
        } = response;
        // Reject contradictory responses atomically instead of guessing precedence.
        if revision.is_empty() || (snapshot.is_some() && delta.is_some()) {
            return ApplyResult::Gap;
        }
        if let Some(snapshot) = snapshot {
            let mut changes = ChangeSet::default();
            changes.changed_message_ids =
                unique(snapshot.messages.iter().map(|row| row.id.clone()));
            changes.changed_channel_ids =
                unique(snapshot.channels.iter().map(|row| row.id.clone()));
            changes.reset_channel_ids = changes.changed_channel_ids.clone();
            changes.changed_subagent_transcript_ids =
                unique(snapshot.subagent_transcripts.keys().cloned());
            if let Some(old) = &self.snapshot {
                changes.removed_message_ids = unique(
                    old.messages
                        .iter()
                        .filter(|row| !snapshot.messages.iter().any(|next| next.id == row.id))
                        .map(|row| row.id.clone()),
                );
                changes.removed_task_ids = unique(
                    old.tasks
                        .iter()
                        .filter(|row| !snapshot.tasks.iter().any(|next| next.id == row.id))
                        .map(|row| row.id.clone()),
                );
                changes.dropped_channel_ids = unique(
                    old.channels
                        .iter()
                        .filter(|row| !snapshot.channels.iter().any(|next| next.id == row.id))
                        .map(|row| row.id.clone()),
                );
                changes.dropped_subagent_transcript_ids = unique(
                    old.subagent_transcripts
                        .keys()
                        .filter(|id| !snapshot.subagent_transcripts.contains_key(*id))
                        .cloned(),
                );
            }
            self.snapshot = Some(snapshot);
            self.revision = Some(revision);
            return ApplyResult::FullReset(changes);
        }
        let Some(delta) = delta else {
            return if self.snapshot.is_some() && self.revision.as_deref() == Some(&revision) {
                ApplyResult::Unchanged
            } else {
                ApplyResult::Gap
            };
        };
        if self.revision.as_deref() != Some(delta.from_revision.as_str()) || self.snapshot.is_none()
        {
            return ApplyResult::Gap;
        }
        let old = self.snapshot.as_ref().unwrap();
        let mut metadata = delta.metadata;
        let channels: BTreeSet<_> = metadata
            .channels
            .iter()
            .map(|row| row.id.as_str())
            .collect();
        let mut seen = BTreeSet::new();
        if delta.channel_message_changes.iter().any(|change| {
            !channels.contains(change.channel_id.as_str())
                || !seen.insert(change.channel_id.as_str())
        }) {
            return ApplyResult::Gap;
        }
        let mut changes = ChangeSet {
            changed_message_ids: unique(delta.messages.iter().map(|row| row.id.clone())),
            removed_message_ids: unique(delta.removed_message_ids.iter().cloned()),
            removed_task_ids: unique(
                old.tasks
                    .iter()
                    .filter(|row| !metadata.tasks.iter().any(|next| next.id == row.id))
                    .map(|row| row.id.clone()),
            ),
            dropped_channel_ids: unique(
                old.channels
                    .iter()
                    .filter(|row| !channels.contains(row.id.as_str()))
                    .map(|row| row.id.clone()),
            ),
            changed_subagent_transcript_ids: unique(metadata.subagent_transcripts.keys().cloned()),
            ..ChangeSet::default()
        };
        metadata.messages = if delta.messages.is_empty() && delta.removed_message_ids.is_empty() {
            old.messages.clone()
        } else {
            merge_messages(
                old.messages.iter().cloned(),
                delta.messages,
                &delta.removed_message_ids,
            )
            .into()
        };
        let retained: BTreeSet<_> = delta.retained_channel_ids.into_iter().collect();
        let mut channel_changes: BTreeMap<_, _> = delta
            .channel_message_changes
            .into_iter()
            .map(|change| (change.channel_id.clone(), change))
            .collect();
        for channel in &mut metadata.channels {
            let previous = old.channels.iter().find(|row| row.id == channel.id);
            if let Some(change) = channel_changes.remove(&channel.id) {
                changes.changed_channel_ids.push(channel.id.clone());
                if change.reset {
                    changes.reset_channel_ids.push(channel.id.clone());
                    channel.messages =
                        merge_channel(std::iter::empty(), change.messages, &[]).into();
                } else {
                    channel.messages = merge_channel(
                        previous
                            .into_iter()
                            .flat_map(|row| row.messages.iter().cloned()),
                        change.messages,
                        &change.removed_message_ids,
                    )
                    .into();
                }
            } else if retained.contains(&channel.id) {
                if let Some(previous) = previous {
                    channel.messages = previous.messages.clone();
                }
            } else {
                changes.changed_channel_ids.push(channel.id.clone());
                changes.reset_channel_ids.push(channel.id.clone());
            }
        }
        for id in delta.retained_subagent_transcript_ids {
            if !metadata.subagent_transcripts.contains_key(&id) {
                if let Some(previous) = old.subagent_transcripts.get(&id) {
                    metadata.subagent_transcripts.insert(id, previous.clone());
                }
            }
        }
        changes.dropped_subagent_transcript_ids = unique(
            old.subagent_transcripts
                .keys()
                .filter(|id| !metadata.subagent_transcripts.contains_key(*id))
                .cloned(),
        );
        changes.changed_channel_ids = unique(changes.changed_channel_ids);
        changes.reset_channel_ids = unique(changes.reset_channel_ids);
        self.snapshot = Some(metadata);
        self.revision = Some(revision);
        ApplyResult::Delta(changes)
    }
    pub fn merge_task_page(
        &mut self,
        task_id: &str,
        page: TaskMessagesPage,
    ) -> Result<ChangeSet, String> {
        if self.revision.as_deref() != Some(page.revision.as_str()) {
            return Err("Task history page is stale; retry at the current revision.".into());
        }
        let snapshot = self
            .snapshot
            .as_mut()
            .ok_or("Native snapshot has not loaded.")?;
        if !snapshot.tasks.iter().any(|task| task.id == task_id)
            || page.messages.iter().any(|row| row.task_id != task_id)
        {
            return Err("Task history page does not match the requested current task.".into());
        }
        // Duplicate IDs owned by another scope must never overwrite its cache.
        if page.messages.iter().any(|row| {
            snapshot
                .messages
                .iter()
                .any(|old| old.id == row.id && old.task_id != task_id)
        }) {
            return Err("Task history page contains an ID from another task.".into());
        }
        let changes = ChangeSet {
            changed_message_ids: unique(page.messages.iter().map(|row| row.id.clone())),
            ..ChangeSet::default()
        };
        if !page.messages.is_empty() {
            snapshot.messages =
                merge_messages(snapshot.messages.iter().cloned(), page.messages, &[]).into();
        }
        Ok(changes)
    }
    pub fn merge_channel_page(
        &mut self,
        channel_id: &str,
        page: ChannelMessagesPage,
    ) -> Result<ChangeSet, String> {
        if self.revision.as_deref() != Some(page.revision.as_str()) {
            return Err("Channel history page is stale; retry at the current revision.".into());
        }
        if page.channel_id != channel_id {
            return Err("Channel history page does not match the requested channel.".into());
        }
        let snapshot = self
            .snapshot
            .as_mut()
            .ok_or("Native snapshot has not loaded.")?;
        if page.messages.iter().any(|message| {
            snapshot.channels.iter().any(|channel| {
                channel.id != channel_id && channel.messages.iter().any(|old| old.id == message.id)
            })
        }) {
            return Err("Channel history page contains an ID from another channel.".into());
        }
        let channel = snapshot
            .channels
            .iter_mut()
            .find(|row| row.id == channel_id)
            .ok_or("Channel no longer exists in the current snapshot.")?;
        if !page.messages.is_empty() {
            channel.messages =
                merge_channel(channel.messages.iter().cloned(), page.messages, &[]).into();
        }
        Ok(ChangeSet {
            changed_channel_ids: vec![channel_id.into()],
            ..ChangeSet::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monitter_lib::{
        experimental_desktop::{ChannelMessageChange, UiDelta},
        model::{Channel, SubagentTranscriptEntry, default_snapshot},
    };
    fn message(id: &str, task: &str, time: i64, text: &str) -> Message {
        Message {
            id: id.into(),
            task_id: task.into(),
            role: "assistant".into(),
            text: text.into(),
            created_at: time,
            stream_status: None,
            phase: None,
            response_metadata: None,
            sender_agent_id: None,
            collaboration_id: None,
            attachments: vec![],
        }
    }
    fn cm(id: &str, time: i64) -> ChannelMessage {
        ChannelMessage {
            id: id.into(),
            role: "assistant".into(),
            agent_id: None,
            text: id.into(),
            created_at: time,
            task_id: None,
        }
    }
    fn fixture() -> Snapshot {
        let mut snapshot = default_snapshot();
        snapshot.tasks.push(serde_json::from_value(serde_json::json!({"id":"t","agentId":"a","title":"Fixture","status":"idle","createdAt":0,"updatedAt":0,"hostId":"h","cwd":"/tmp","provider":"codex","model":"test","sandbox":"read-only"})).unwrap());
        snapshot.messages = vec![message("tail", "t", 10, "old")].into();
        snapshot.channels = vec![Channel {
            id: "c".into(),
            name: "Channel".into(),
            description: String::new(),
            agent_ids: vec![],
            messages: vec![cm("ctail", 10)].into(),
            agent_conversation_enabled: false,
            agent_conversation_turn_limit: 3,
            agent_conversation_turns_used: 0,
            agent_conversation_paused: false,
        }];
        snapshot.subagent_transcripts.insert(
            "s".into(),
            vec![SubagentTranscriptEntry {
                id: "entry".into(),
                role: "assistant".into(),
                text: "retained".into(),
                created_at: 0,
            }]
            .into(),
        );
        snapshot
    }
    fn state() -> NativeState {
        let mut state = NativeState::default();
        assert!(matches!(
            state.apply(UiDeltaResponse {
                revision: "e:1".into(),
                snapshot: Some(fixture()),
                delta: None
            }),
            ApplyResult::FullReset(_)
        ));
        state
    }
    fn delta(state: &NativeState) -> UiDelta {
        let mut metadata = state.snapshot().unwrap().clone();
        metadata.messages.clear();
        metadata.channels[0].messages.clear();
        metadata.subagent_transcripts.clear();
        UiDelta {
            from_revision: "e:1".into(),
            metadata,
            messages: vec![],
            removed_message_ids: vec![],
            retained_channel_ids: vec!["c".into()],
            channel_message_changes: vec![],
            retained_subagent_transcript_ids: vec!["s".into()],
        }
    }
    fn response(delta: UiDelta) -> UiDeltaResponse {
        UiDeltaResponse {
            revision: "e:2".into(),
            snapshot: None,
            delta: Some(delta),
        }
    }
    #[::core::prelude::v1::test]
    fn unchanged_and_gap_never_mutate_cache() {
        let mut state = state();
        let before = state.snapshot().unwrap().clone();
        assert_eq!(
            state.apply(UiDeltaResponse {
                revision: "e:1".into(),
                snapshot: None,
                delta: None
            }),
            ApplyResult::Unchanged
        );
        let mut bad = delta(&state);
        bad.from_revision = "e:0".into();
        assert_eq!(state.apply(response(bad)), ApplyResult::Gap);
        assert_eq!(state.snapshot().unwrap(), &before);
        assert_eq!(state.revision(), Some("e:1"));
    }
    #[::core::prelude::v1::test]
    fn ui_page_eviction_keeps_other_scopes_revision_and_reloadability() {
        let mut state = state();
        state.merge_task_page("t", TaskMessagesPage {
            revision: "e:1".into(),
            messages: vec![message("older", "t", 1, "older")],
            next_before_id: None,
        }).unwrap();
        let revision = state.revision().unwrap().to_owned();
        assert!(state.trim_task_messages("t", 1));
        assert_eq!(state.snapshot().unwrap().messages.len(), 1);
        assert_eq!(state.snapshot().unwrap().messages[0].id, "tail");
        assert_eq!(state.snapshot().unwrap().channels[0].messages[0].id, "ctail");
        assert_eq!(state.revision(), Some(revision.as_str()));
        state.merge_task_page("t", TaskMessagesPage {
            revision,
            messages: vec![message("older", "t", 1, "older")],
            next_before_id: None,
        }).unwrap();
        assert_eq!(state.snapshot().unwrap().messages.len(), 2);
    }

    #[::core::prelude::v1::test]
    fn loaded_older_history_survives_delta_and_updates_dedupe() {
        let mut state = state();
        state
            .merge_task_page(
                "t",
                TaskMessagesPage {
                    revision: "e:1".into(),
                    messages: vec![message("older", "t", 1, "older")],
                    next_before_id: None,
                },
            )
            .unwrap();
        let mut edit = delta(&state);
        edit.messages = vec![
            message("tail", "t", 10, "updated"),
            message("new", "t", 11, "new"),
        ];
        let result = state.apply(response(edit));
        assert!(
            matches!(result,ApplyResult::Delta(ref changed) if changed.changed_message_ids==vec!["new","tail"])
        );
        let values: Vec<_> = state
            .snapshot()
            .unwrap()
            .messages
            .iter()
            .map(|row| (row.id.as_str(), row.text.as_str()))
            .collect();
        assert_eq!(
            values,
            vec![("older", "older"), ("tail", "updated"), ("new", "new")]
        );
    }
    #[::core::prelude::v1::test]
    fn explicit_removal_wins_over_conflicting_upsert() {
        let mut state = state();
        let mut edit = delta(&state);
        edit.removed_message_ids = vec!["tail".into()];
        edit.messages = vec![message("tail", "t", 10, "do not restore")];
        assert!(
            matches!(state.apply(response(edit)),ApplyResult::Delta(ref changed) if changed.removed_message_ids==vec!["tail"])
        );
        assert!(state.snapshot().unwrap().messages.is_empty());
    }
    #[::core::prelude::v1::test]
    fn full_reset_drops_loaded_pages_and_reports_removed_scopes() {
        let mut state = state();
        state
            .merge_task_page(
                "t",
                TaskMessagesPage {
                    revision: "e:1".into(),
                    messages: vec![message("older", "t", 1, "loaded")],
                    next_before_id: None,
                },
            )
            .unwrap();
        let mut fresh = fixture();
        fresh.tasks.clear();
        fresh.messages.clear();
        fresh.channels.clear();
        fresh.subagent_transcripts.clear();
        let result = state.apply(UiDeltaResponse {
            revision: "other:0".into(),
            snapshot: Some(fresh),
            delta: None,
        });
        assert!(
            matches!(result,ApplyResult::FullReset(ref changed) if changed.removed_task_ids==vec!["t"]&&changed.dropped_channel_ids==vec!["c"]&&changed.dropped_subagent_transcript_ids==vec!["s"])
        );
        assert!(state.snapshot().unwrap().messages.is_empty());
    }
    #[::core::prelude::v1::test]
    fn channel_merge_preserves_older_and_reset_discards_it() {
        let mut state = state();
        state
            .merge_channel_page(
                "c",
                ChannelMessagesPage {
                    channel_id: "c".into(),
                    revision: "e:1".into(),
                    messages: vec![cm("older", 1)],
                    next_before_id: None,
                },
            )
            .unwrap();
        let mut edit = delta(&state);
        edit.retained_channel_ids.clear();
        edit.channel_message_changes = vec![ChannelMessageChange {
            channel_id: "c".into(),
            messages: vec![cm("new", 11)],
            removed_message_ids: vec!["ctail".into()],
            reset: false,
        }];
        state.apply(response(edit));
        assert_eq!(
            state.snapshot().unwrap().channels[0]
                .messages
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["older", "new"]
        );
        let mut edit = delta(&state);
        edit.from_revision = "e:2".into();
        edit.channel_message_changes = vec![ChannelMessageChange {
            channel_id: "c".into(),
            messages: vec![cm("reset", 20)],
            removed_message_ids: vec![],
            reset: true,
        }];
        let result = state.apply(UiDeltaResponse {
            revision: "e:3".into(),
            snapshot: None,
            delta: Some(edit),
        });
        assert!(
            matches!(result,ApplyResult::Delta(ref changed) if changed.reset_channel_ids==vec!["c"])
        );
        assert_eq!(state.snapshot().unwrap().channels[0].messages.len(), 1);
        assert_eq!(
            state.snapshot().unwrap().channels[0].messages[0].id,
            "reset"
        );
    }
    #[::core::prelude::v1::test]
    fn retained_subagents_survive_and_missing_unretained_are_dropped() {
        let mut state = state();
        state.apply(response(delta(&state)));
        assert_eq!(
            state.snapshot().unwrap().subagent_transcripts["s"][0].text,
            "retained"
        );
        let mut edit = delta(&state);
        edit.from_revision = "e:2".into();
        edit.retained_subagent_transcript_ids.clear();
        let result = state.apply(UiDeltaResponse {
            revision: "e:3".into(),
            snapshot: None,
            delta: Some(edit),
        });
        assert!(
            matches!(result,ApplyResult::Delta(ref changed) if changed.dropped_subagent_transcript_ids==vec!["s"])
        );
    }
    #[::core::prelude::v1::test]
    fn scoped_pages_reject_stale_mixed_and_wrong_channel_atomically() {
        let mut state = state();
        let before = state.snapshot().unwrap().clone();
        assert!(
            state
                .merge_task_page(
                    "t",
                    TaskMessagesPage {
                        revision: "old".into(),
                        messages: vec![],
                        next_before_id: None
                    }
                )
                .is_err()
        );
        assert!(
            state
                .merge_task_page(
                    "t",
                    TaskMessagesPage {
                        revision: "e:1".into(),
                        messages: vec![
                            message("ok", "t", 1, "ok"),
                            message("bad", "other", 2, "bad")
                        ],
                        next_before_id: None
                    }
                )
                .is_err()
        );
        assert!(
            state
                .merge_channel_page(
                    "c",
                    ChannelMessagesPage {
                        channel_id: "wrong".into(),
                        revision: "e:1".into(),
                        messages: vec![cm("bad", 1)],
                        next_before_id: None
                    }
                )
                .is_err()
        );
        assert_eq!(state.snapshot().unwrap(), &before);
    }
    #[::core::prelude::v1::test]
    fn contradictory_response_and_unknown_channel_delta_leave_state_intact() {
        let mut state = state();
        let before = state.snapshot().unwrap().clone();
        let edit = delta(&state);
        assert_eq!(
            state.apply(UiDeltaResponse {
                revision: "e:2".into(),
                snapshot: Some(fixture()),
                delta: Some(edit)
            }),
            ApplyResult::Gap
        );
        let mut edit = delta(&state);
        edit.channel_message_changes = vec![ChannelMessageChange {
            channel_id: "unknown".into(),
            messages: vec![cm("wrong", 1)],
            removed_message_ids: vec![],
            reset: true,
        }];
        assert_eq!(state.apply(response(edit)), ApplyResult::Gap);
        assert_eq!(state.snapshot().unwrap(), &before);
        assert_eq!(state.revision(), Some("e:1"));
    }
}
