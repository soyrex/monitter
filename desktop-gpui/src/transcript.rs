//! Native variable-height history with stable message identity and reader anchors.
use crate::design::ui_scaled_px as px;
#[path = "transcript_grouping.rs"]
mod transcript_grouping;
use gpui_kit::{FollowMode, ListAlignment, ListOffset, ListState};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};
pub(crate) use transcript_grouping::reasoning_summary;
pub use transcript_grouping::{
    ApprovalRow, ConversationItem, NestedGroup, project as project_conversation,
    project_with_options as project_conversation_with_options,
    visible_event_ids as visible_projection_event_ids,
};

fn id(message: &Value) -> &str {
    message["id"].as_str().unwrap_or_default()
}
/// Native render projection only; message cache and its paging cursor remain
/// independent from the compact authoritative activity metadata.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineRow {
    Message(Rc<Value>),
    Activity(Rc<Value>),
}
impl TimelineRow {
    pub fn key(&self) -> String {
        match self {
            Self::Message(row) => format!("message:{}", id(row)),
            Self::Activity(row) => format!("event:{}", id(row)),
        }
    }
    pub fn at(&self) -> i64 {
        match self {
            Self::Message(row) | Self::Activity(row) => row["createdAt"].as_i64().unwrap_or(0),
        }
    }
}
fn timeline(messages: &[Rc<Value>], activities: &[Rc<Value>]) -> Vec<TimelineRow> {
    let mut rows: Vec<_> = messages
        .iter()
        .cloned()
        .map(TimelineRow::Message)
        .chain(activities.iter().cloned().map(TimelineRow::Activity))
        .collect();
    rows.sort_by(|a, b| a.at().cmp(&b.at()).then_with(|| a.key().cmp(&b.key())));
    rows
}
#[derive(Default)]
pub enum HistoryBoundary {
    #[default]
    Unknown,
    More(String),
    Complete,
}
enum PendingLive {
    Merge(Vec<Rc<Value>>),
    Replace(Vec<Rc<Value>>),
}
pub struct TranscriptState {
    pub list: ListState,
    pub messages: Rc<Vec<Rc<Value>>>,
    pub rows: Rc<Vec<TimelineRow>>,
    /// Compact render projection. `messages` and `rows` remain authoritative.
    pub projection: Rc<Vec<ConversationItem>>,
    compress_tools: bool,
    projection_source: Option<Rc<Vec<TimelineRow>>>,
    projection_approvals: Vec<ApprovalRow>,
    activities: Vec<Rc<Value>>,
    last_activities: Vec<Rc<Value>>,
    pending_activities: Option<Vec<Rc<Value>>>,
    last_compact: Vec<Rc<Value>>,
    pub boundary: HistoryBoundary,
    /// True only after user scrolling reaches the first loaded row, away from tail-follow.
    pub history_top_reached: bool,
    pub loading: bool,
    pub error: Option<String>,
    pending_live: Option<PendingLive>,
}
impl Default for TranscriptState {
    fn default() -> Self {
        let list = ListState::new(0, ListAlignment::Bottom, px(300.));
        // GPUI's explicit tail state remains reliable while virtualized rows
        // have unknown heights; `is_scrolled_to_end()` returns None then.
        list.set_follow_mode(FollowMode::Tail);
        Self {
            list,
            messages: Rc::new(vec![]),
            rows: Rc::new(vec![]),
            projection: Rc::new(vec![]),
            compress_tools: false,
            projection_source: None,
            projection_approvals: vec![],
            activities: vec![],
            last_activities: vec![],
            pending_activities: None,
            last_compact: vec![],
            boundary: HistoryBoundary::Unknown,
            history_top_reached: false,
            loading: false,
            error: None,
            pending_live: None,
        }
    }
}
impl TranscriptState {
    /// Evict only a tail-following inactive reader. An off-tail anchor, held
    /// live update, or in-flight page always keeps its existing history intact.
    pub fn can_trim_inactive(&self) -> bool {
        self.list.is_following_tail() && !self.loading && !self.has_pending_live()
    }
    pub fn trim_inactive_tail(&mut self, keep: usize) -> bool {
        if !self.can_trim_inactive() || self.messages.len() <= keep || keep == 0 {
            return false;
        }
        let retained = self.messages[self.messages.len() - keep..].to_vec();
        self.boundary = HistoryBoundary::More(id(&retained[0]).to_owned());
        self.last_compact = retained.clone();
        self.update_shared(retained);
        self.history_top_reached = false;
        true
    }
    pub fn has_pending_live(&self) -> bool {
        self.pending_live.is_some() || self.pending_activities.is_some()
    }
    pub fn before_id(&self) -> Option<String> {
        match &self.boundary {
            HistoryBoundary::More(cursor) => Some(cursor.clone()),
            _ => self.messages.first().map(|message| id(message).to_owned()),
        }
    }
    pub fn complete(&self) -> bool {
        matches!(self.boundary, HistoryBoundary::Complete)
    }
    /// Derive the grouped conversation projection from the current flat rows.
    /// The grouped view reuses the underlying event rows so disclosure state
    /// in [`super::run_activity::RunActivityRow`] survives projection rebuilds.
    /// Approval rows are forwarded from the caller so resolved approvals can
    /// appear inline without leaking pending request state.
    pub fn project(&self, approvals: &[ApprovalRow]) -> Vec<ConversationItem> {
        project_conversation_with_options(&self.rows, approvals, self.compress_tools)
    }
    /// Change whether tool calls are grouped in the next native projection.
    /// The caller follows this with `sync_projection` during its render pass.
    pub fn set_compress_tools(&mut self, enabled: bool) -> bool {
        if self.compress_tools == enabled {
            return false;
        }
        self.compress_tools = enabled;
        self.projection_source = None;
        true
    }
    /// Reconcile virtual-list indices against the visible grouped projection.
    /// This keeps the reader anchored to the same first member when groups
    /// grow, split, or gain an approval row.
    pub fn sync_projection(&mut self, approvals: &[ApprovalRow]) {
        // Input and decorative invalidations do not change transcript data.
        // Reuse the projection without cloning/grouping all history again.
        if self.projection_source.as_ref().is_some_and(|source| Rc::ptr_eq(source, &self.rows))
            && self.projection_approvals == approvals
        {
            return;
        }
        self.projection_source = Some(self.rows.clone());
        self.projection_approvals = approvals.to_vec();
        let next = self.project(approvals);
        let previous = self.projection.as_ref();
        // Root/settings/activity invalidations re-enter this function even if
        // the projection is unchanged. Do not issue a scroll command in that
        // case: it would override an in-progress native wheel gesture.
        if previous.as_slice() == next.as_slice() {
            return;
        }
        let following = self.list.is_following_tail();
        let offset = self.list.logical_scroll_top();
        let anchor = previous
            .get(offset.item_ix)
            .map(ConversationItem::stable_key);
        let mut prefix = 0;
        while prefix < previous.len().min(next.len())
            && previous[prefix].stable_key() == next[prefix].stable_key()
        {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < previous.len().min(next.len()) - prefix
            && previous[previous.len() - 1 - suffix].stable_key()
                == next[next.len() - 1 - suffix].stable_key()
        {
            suffix += 1;
        }
        if prefix + suffix != previous.len() || prefix + suffix != next.len() {
            self.list.splice(
                prefix..previous.len() - suffix,
                next.len() - prefix - suffix,
            );
        }
        let old: HashMap<String, &ConversationItem> = previous
            .iter()
            .map(|item| (item.stable_key(), item))
            .collect();
        for (index, item) in next.iter().enumerate() {
            if old
                .get(&item.stable_key())
                .is_some_and(|prior| **prior != *item)
            {
                self.list.splice(index..index + 1, 1);
            }
        }
        self.projection = Rc::new(next);
        if !following {
            if let Some(index) = anchor.and_then(|key| {
                self.projection
                    .iter()
                    .position(|item| item.stable_key() == key)
            }) {
                self.list.scroll_to(ListOffset {
                    item_ix: index,
                    offset_in_item: offset.offset_in_item,
                });
            }
        } else {
            self.list.scroll_to_end();
        }
    }
    pub fn merge_compact(&mut self, latest: Vec<Value>) {
        let latest = latest.into_iter().map(Rc::new).collect::<Vec<_>>();
        if self.last_compact == latest {
            return;
        }
        self.last_compact = latest.clone();
        if !self.messages.is_empty() && !self.list.is_following_tail() {
            self.pending_live = Some(PendingLive::Merge(latest));
            return;
        }
        self.merge_latest(latest);
    }
    pub fn release_live(&mut self) {
        let activity_changed = if let Some(activities) = self.pending_activities.take() {
            self.activities = activities;
            true
        } else {
            false
        };
        match self.pending_live.take() {
            Some(PendingLive::Merge(latest)) => self.merge_latest(latest),
            Some(PendingLive::Replace(latest)) => self.update_shared(latest),
            None => {
                if activity_changed {
                    self.update_shared((*self.messages).clone());
                }
            }
        }
    }
    pub fn reconcile_activities(&mut self, activities: Vec<Value>, reset: bool) {
        let activities = activities.into_iter().map(Rc::new).collect::<Vec<_>>();
        if reset {
            self.pending_activities = None;
            self.last_activities = activities.clone();
            self.activities = activities;
            self.update_shared((*self.messages).clone());
        } else if self.last_activities != activities {
            self.last_activities = activities.clone();
            if !self.rows.is_empty() && !self.list.is_following_tail() {
                self.pending_activities = Some(activities);
            } else {
                self.pending_activities = None;
                self.activities = activities;
                self.update_shared((*self.messages).clone());
            }
        }
    }
    /// Reconcile the native cache's entire known task history, including loaded
    /// pages. Unlike a compact tail, this list is authoritative for removals.
    /// A journal reset invalidates page cursors and any held prior revision.
    pub fn reconcile(&mut self, messages: Vec<Value>, reset: bool) {
        let messages = messages.into_iter().map(Rc::new).collect::<Vec<_>>();
        if reset {
            self.boundary = HistoryBoundary::Unknown;
            self.loading = false;
            self.error = None;
            self.pending_live = None;
            self.pending_activities = None;
            self.activities.clear();
            self.last_activities.clear();
            self.last_compact = messages.clone();
            self.update_shared(messages);
        } else if self.last_compact != messages {
            if let HistoryBoundary::More(cursor) = &self.boundary {
                if !messages.iter().any(|message| id(message) == cursor) {
                    self.boundary = messages
                        .first()
                        .map_or(HistoryBoundary::Unknown, |message| {
                            HistoryBoundary::More(id(message).to_owned())
                        });
                }
            }
            self.last_compact = messages.clone();
            if !self.messages.is_empty() && !self.list.is_following_tail() {
                self.pending_live = Some(PendingLive::Replace(messages));
            } else {
                self.pending_live = None;
                self.update_shared(messages);
            }
        }
    }
    fn merge_latest(&mut self, latest: Vec<Rc<Value>>) {
        let mut merged = self.messages.as_ref().clone();
        let indices: HashMap<String, usize> = merged
            .iter()
            .enumerate()
            .map(|(index, message)| (id(message).to_owned(), index))
            .collect();
        for message in latest {
            if let Some(index) = indices.get(id(&message)) {
                merged[*index] = message;
            } else {
                merged.push(message);
            }
        }
        self.update_shared(merged);
    }
    pub fn merge_page(&mut self, page: &Value) {
        let mut merged: Vec<_> = page["messages"].as_array().into_iter().flatten().cloned().map(Rc::new).collect();
        let ids: HashSet<String> = merged
            .iter()
            .map(|message| id(message).to_owned())
            .collect();
        merged.extend(
            self.messages
                .iter()
                .filter(|message| !ids.contains(id(message)))
                .cloned(),
        );
        self.boundary = match page["nextBeforeId"].as_str() {
            Some(cursor) => HistoryBoundary::More(cursor.to_owned()),
            None => HistoryBoundary::Complete,
        };
        self.update_shared(merged);
    }
    pub(crate) fn update(&mut self, messages: Vec<Value>) {
        self.update_shared(messages.into_iter().map(Rc::new).collect());
    }
    fn update_shared(&mut self, messages: Vec<Rc<Value>>) {
        let rows = timeline(&messages, &self.activities);
        self.messages = Rc::new(messages);
        self.rows = Rc::new(rows);
        self.sync_projection(&[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    // Allow the grouping projection to be referenced from test code without
    // pulling the public api into the renderer's call path.

    #[::core::prelude::v1::test]
    fn inactive_tail_eviction_restores_cursor_and_protects_held_reader() {
        let mut state = TranscriptState::default();
        let messages = (0..100).map(|n| json!({"id":format!("m{n}"),"role":"user","createdAt":n,"text":"history"})).collect();
        state.reconcile(messages, true);
        assert!(state.trim_inactive_tail(10));
        assert_eq!(state.messages.len(), 10);
        assert_eq!(state.before_id().as_deref(), Some("m90"));
        assert!(!state.complete());
        state.pending_live = Some(PendingLive::Merge(vec![Rc::new(json!({"id":"held"}))]));
        assert!(!state.trim_inactive_tail(5));
        assert_eq!(state.messages.len(), 10);
        assert!(state.has_pending_live());
        state.pending_live = None;
        state.list.set_follow_mode(FollowMode::Normal);
        assert!(!state.trim_inactive_tail(5));
        assert_eq!(state.messages.len(), 10);
        assert_eq!(state.before_id().as_deref(), Some("m90"));
    }

    #[::core::prelude::v1::test]
    fn repeated_input_renders_reuse_history_but_messages_and_approvals_refresh() {
        let mut state = TranscriptState::default();
        state.update(vec![json!({"id":"m", "role":"user", "text":"before", "createdAt":1})]);
        let initial = state.projection.clone();
        for _ in 0..100 {
            state.sync_projection(&[]);
        }
        assert!(Rc::ptr_eq(&initial, &state.projection));
        state.update(vec![json!({"id":"m", "role":"user", "text":"after", "createdAt":1})]);
        assert!(!Rc::ptr_eq(&initial, &state.projection));
        assert!(matches!(&state.projection[0], ConversationItem::Message(value) if value["text"] == "after"));
        let approval = ApprovalRow {
            id: "approval".into(), task_id: "task".into(), status: "approved".into(),
            created_at: 2, resolved_at: Some(3), rule_id: None,
            summary: "Allowed".into(), tool: "exec".into(),
        };
        state.sync_projection(&[approval]);
        assert_eq!(state.projection.len(), 2);
        state.sync_projection(&[]);
        assert_eq!(state.projection.len(), 1);
    }

    #[::core::prelude::v1::test]
    fn authoritative_history_removes_deleted_rows_and_reset_invalidates_cursor() {
        let mut state = TranscriptState::default();
        state.merge_page(&json!({"messages":[{"id":"old","text":"old"},{"id":"live","text":"before"}],"nextBeforeId":"old"}));
        state.reconcile(vec![json!({"id":"live","text":"updated"})], false);
        state.release_live();
        assert_eq!(&*state.messages, &[Rc::new(json!({"id":"live","text":"updated"}))]);
        assert_eq!(state.before_id().as_deref(), Some("live"));
        state.loading = true;
        state.error = Some("obsolete".into());
        state.reconcile(vec![json!({"id":"reset","text":"new epoch"})], true);
        assert!(matches!(state.boundary, HistoryBoundary::Unknown));
        assert!(!state.loading && state.error.is_none() && !state.has_pending_live());
        assert_eq!(state.before_id().as_deref(), Some("reset"));
    }
    #[::core::prelude::v1::test]
    fn timeline_orders_activity_between_messages_without_identity_collision() {
        let rows = timeline(
            &[
                Rc::new(json!({"id":"same","createdAt":10})),
                Rc::new(json!({"id":"later","createdAt":30})),
            ],
            &[Rc::new(json!({"id":"same","createdAt":20}))],
        );
        assert_eq!(
            rows.iter().map(TimelineRow::key).collect::<Vec<_>>(),
            ["message:same", "event:same", "message:later"]
        );
    }

    #[::core::prelude::v1::test]
    fn held_activity_and_messages_release_together_without_changing_page_cursor() {
        let mut state = TranscriptState::default();
        state.reconcile(vec![json!({"id":"old","createdAt":10})], true);
        state.boundary = HistoryBoundary::More("older-page".into());
        state.pending_live = Some(PendingLive::Replace(vec![
            Rc::new(json!({"id":"old","createdAt":10})),
            Rc::new(json!({"id":"new","createdAt":30})),
        ]));
        state.pending_activities = Some(vec![Rc::new(json!({"id":"tool","createdAt":20}))]);
        assert_eq!(state.rows.len(), 1);
        state.release_live();
        assert_eq!(
            state.rows.iter().map(TimelineRow::key).collect::<Vec<_>>(),
            ["message:old", "event:tool", "message:new"]
        );
        assert!(!state.has_pending_live());
        assert_eq!(state.before_id().as_deref(), Some("older-page"));
    }

    #[::core::prelude::v1::test]
    fn reset_discards_held_activity_and_does_not_use_event_as_message_cursor() {
        let mut state = TranscriptState::default();
        state.reconcile(vec![json!({"id":"message","createdAt":20})], true);
        state.reconcile_activities(vec![json!({"id":"event","createdAt":10})], true);
        assert_eq!(state.before_id().as_deref(), Some("message"));
        state.pending_activities = Some(vec![Rc::new(json!({"id":"stale","createdAt":30}))]);
        state.reconcile(vec![json!({"id":"reset","createdAt":40})], true);
        state.release_live();
        assert_eq!(
            state.rows.iter().map(TimelineRow::key).collect::<Vec<_>>(),
            ["message:reset"]
        );
    }

    #[::core::prelude::v1::test]
    fn same_event_identity_updates_payload_and_authoritative_removal_removes_row() {
        let mut state = TranscriptState::default();
        state.reconcile_activities(
            vec![json!({"id":"event","createdAt":10,"title":"before"})],
            true,
        );
        state.reconcile_activities(
            vec![json!({"id":"event","createdAt":10,"title":"after"})],
            true,
        );
        assert!(
            matches!(&state.rows[0], TimelineRow::Activity(event) if event["title"] == "after")
        );
        state.reconcile_activities(vec![], true);
        assert!(state.rows.is_empty());
    }

    #[::core::prelude::v1::test]
    fn project_collapses_reasoning_and_tool_groups_from_flat_rows() {
        let mut state = TranscriptState::default();
        state.reconcile(
            vec![json!({"id":"msg","role":"user","taskId":"task-1","createdAt":5,"text":"hi"})],
            true,
        );
        state.reconcile_activities(
            vec![
                json!({"id":"a","kind":"reasoning","taskId":"task-1","createdAt":10,"title":"Thinking","detail":"{\"summary\":\"one\"}"}),
                json!({"id":"b","kind":"reasoning","taskId":"task-1","createdAt":15,"title":"Thinking","detail":"{\"summary\":\"two\"}"}),
                json!({"id":"c","kind":"tool","taskId":"task-1","createdAt":20,"title":"Read","detail":"{\"kind\":\"read\"}"}),
            ],
            true,
        );
        let projection = state.project(&[]);
        // Adjacent reasoning and tool activity form one process disclosure.
        assert_eq!(projection.len(), 2);
        assert_eq!(projection[0].stable_key(), "message:msg");
        assert_eq!(projection[1].stable_key(), "process-group:a");
        assert_eq!(projection[1].row_count(), 3);
    }

    #[::core::prelude::v1::test]
    fn project_holds_first_event_ids_across_live_update_and_reset() {
        let mut state = TranscriptState::default();
        state.reconcile_activities(
            vec![json!({"id":"a","kind":"reasoning","taskId":"task-1","createdAt":10,"title":"Thinking","detail":"{\"summary\":\"one\"}"}),
                 json!({"id":"b","kind":"reasoning","taskId":"task-1","createdAt":15,"title":"Thinking","detail":"{\"summary\":\"two\"}"}),
            ],
            true,
        );
        let first = state.project(&[]);
        assert_eq!(first[0].stable_key(), "reasoning-group:a");
        // A live append must keep the original anchor stable.
        state.reconcile_activities(
            vec![json!({"id":"c","kind":"reasoning","taskId":"task-1","createdAt":18,"title":"Thinking","detail":"{\"summary\":\"three\"}"}),
                 json!({"id":"a","kind":"reasoning","taskId":"task-1","createdAt":10,"title":"Thinking","detail":"{\"summary\":\"one\"}"}),
                 json!({"id":"b","kind":"reasoning","taskId":"task-1","createdAt":15,"title":"Thinking","detail":"{\"summary\":\"two\"}"}),
            ],
            false,
        );
        let after_live = state.project(&[]);
        assert_eq!(after_live[0].stable_key(), first[0].stable_key());
        assert_eq!(after_live[0].row_count(), 3);
        // Authoritative removal resets the projection; the previous anchor is
        // no longer represented and the new first event takes the slot.
        state.reconcile_activities(
            vec![json!({"id":"only","kind":"reasoning","taskId":"task-1","createdAt":5,"title":"Thinking","detail":"{\"summary\":\"only\"}"}),
            ],
            true,
        );
        let after_reset = state.project(&[]);
        assert_eq!(after_reset[0].stable_key(), "reasoning-group:only");
        assert_eq!(after_reset[0].row_count(), 1);
    }

    #[::core::prelude::v1::test]
    fn project_merges_paginated_older_rows_without_dropping_open_group() {
        let mut state = TranscriptState::default();
        state.reconcile(
            vec![
                json!({"id":"user","role":"user","taskId":"task-1","createdAt":5,"text":"hi"}),
                json!({"id":"assistant","role":"assistant","taskId":"task-1","createdAt":40,"text":"ok","phase":"final_answer"}),
            ],
            true,
        );
        state.reconcile_activities(
            vec![json!({"id":"newest","kind":"reasoning","taskId":"task-1","createdAt":30,"title":"Thinking","detail":"{\"summary\":\"latest\"}"}),
            ],
            true,
        );
        let before_page = state.project(&[]);
        assert_eq!(before_page.len(), 3);
        assert_eq!(before_page[1].stable_key(), "reasoning-group:newest");
        assert_eq!(
            before_page.last().unwrap().stable_key(),
            "message:assistant"
        );
        // An older message page must retain the current reasoning group and
        // keep the final answer after it.
        state.merge_page(&json!({
            "messages":[{"id":"older","role":"assistant","taskId":"task-1","createdAt":20,"text":"prior"}],
            "nextBeforeId":"older",
        }));
        let after_page = state.project(&[]);
        assert_eq!(
            after_page
                .iter()
                .find(|item| item.stable_key() == "reasoning-group:newest")
                .unwrap()
                .stable_key(),
            "reasoning-group:newest"
        );
        assert_eq!(after_page.last().unwrap().stable_key(), "message:assistant");
        assert!(
            after_page
                .iter()
                .any(|item| item.stable_key() == "message:older")
        );
    }

    #[::core::prelude::v1::test]
    fn visible_projection_event_ids_lets_callers_detach_removed_rows() {
        let mut state = TranscriptState::default();
        state.reconcile_activities(
            vec![
                json!({"id":"a","kind":"reasoning","taskId":"task-1","createdAt":10,"title":"Thinking","detail":"{\"summary\":\"one\"}"}),
                json!({"id":"b","kind":"tool","taskId":"task-1","createdAt":20,"title":"Read","detail":"{\"kind\":\"read\"}"}),
            ],
            true,
        );
        let projection = state.project(&[]);
        let ids = visible_projection_event_ids(&projection);
        assert!(ids.contains("a"));
        assert!(ids.contains("b"));
        // Authoritative reset should not return the original ids.
        state.reconcile_activities(
            vec![json!({"id":"c","kind":"reasoning","taskId":"task-1","createdAt":5,"title":"Thinking","detail":"{\"summary\":\"only\"}"}),
            ],
            true,
        );
        let reset_projection = state.project(&[]);
        let reset_ids = visible_projection_event_ids(&reset_projection);
        assert!(!reset_ids.contains("a"));
        assert!(!reset_ids.contains("b"));
        assert!(reset_ids.contains("c"));
    }

    #[::core::prelude::v1::test]
    fn grouped_projection_count_and_first_member_anchor_survive_append() {
        let mut state = TranscriptState::default();
        let event = |id: &str, at| json!({"id":id,"kind":"reasoning","taskId":"task-1","createdAt":at,"title":"Thinking","detail":"{\"summary\":\"working\"}"});
        state.reconcile_activities(vec![event("r1", 1), event("r2", 2)], true);
        let first = state.projection.first().unwrap();
        assert_eq!(first.stable_key(), "reasoning-group:r1");
        assert_eq!(first.row_count(), 2);
        assert_eq!(state.projection.len(), 1);

        state.reconcile_activities(vec![event("r1", 1), event("r2", 2), event("r3", 3)], false);
        let after = state.projection.first().unwrap();
        assert_eq!(after.stable_key(), "reasoning-group:r1");
        assert_eq!(after.row_count(), 3);
        assert_eq!(state.projection.len(), 1);
    }
}
