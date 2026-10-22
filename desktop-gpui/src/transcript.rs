//! Native variable-height history with stable message identity and reader anchors.
use crate::design::ui_scaled_px as px;
#[path = "transcript_grouping.rs"]
mod transcript_grouping;
#[path = "transcript_exchanges.rs"]
mod transcript_exchanges;
pub use transcript_exchanges::ExchangeContext;
use gpui_kit::{FollowMode, ListAlignment, ListOffset, ListState};
use im::Vector;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};
pub(crate) use transcript_grouping::{ContextCompaction, context_compaction, context_compaction_boundary_at, context_compaction_running, is_context_compaction, reasoning_summary};
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
fn timeline<'a>(
    messages: impl IntoIterator<Item = &'a Rc<Value>>,
    activities: &[Rc<Value>],
) -> Vec<TimelineRow> {
    let mut rows: Vec<_> = messages
        .into_iter()
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
    pub messages: Rc<Vector<Rc<Value>>>,
    pub rows: Rc<Vector<TimelineRow>>,
    /// Compact render projection. `messages` and `rows` remain authoritative.
    pub projection: Rc<Vector<ConversationItem>>,
    body_slots: HashMap<String, (usize, usize, usize)>,
    compress_tools: bool,
    exchange_context: Option<ExchangeContext>,
    exchange_boundaries: HashSet<String>,
    exchange_following: bool,
    projection_source: Option<Rc<Vector<TimelineRow>>>,
    projection_approvals: Vec<ApprovalRow>,
    activities: Vec<Rc<Value>>,
    last_activities: Vec<Rc<Value>>,
    pending_activities: Option<Vec<Rc<Value>>>,
    last_compact: Vector<Rc<Value>>,
    /// Built once for a held authoritative history, then reused for streamed
    /// point updates. `None` slots reject ambiguous duplicate message IDs.
    compact_body_slots: Option<HashMap<String, Option<usize>>>,
    held_body_patches: HashMap<String, Rc<Value>>,
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
            messages: Rc::new(Vector::new()),
            rows: Rc::new(Vector::new()),
            projection: Rc::new(Vector::new()),
            body_slots: HashMap::new(),
            compress_tools: false,
            exchange_context: None,
            exchange_boundaries: HashSet::new(),
            exchange_following: true,
            projection_source: None,
            projection_approvals: vec![],
            activities: vec![],
            last_activities: vec![],
            pending_activities: None,
            last_compact: Vector::new(),
            compact_body_slots: None,
            held_body_patches: HashMap::new(),
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
        let retained = self
            .messages
            .iter()
            .skip(self.messages.len() - keep)
            .cloned()
            .collect::<Vec<_>>();
        self.boundary = HistoryBoundary::More(id(&retained[0]).to_owned());
        self.replace_compact(retained.iter().cloned().collect());
        self.update_shared(retained);
        self.history_top_reached = false;
        true
    }
    pub fn has_pending_live(&self) -> bool {
        self.pending_live.is_some()
            || self.pending_activities.is_some()
            || !self.held_body_patches.is_empty()
    }
    pub fn before_id(&self) -> Option<String> {
        match &self.boundary {
            HistoryBoundary::More(cursor) => Some(cursor.clone()),
            _ => self.messages.front().map(|message| id(message).to_owned()),
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
        let items=project_conversation_with_options(self.rows.iter(), approvals, self.compress_tools);
        match &self.exchange_context {Some(context)=>transcript_exchanges::group_with_boundaries(items,context,&self.exchange_boundaries),None=>items}
    }
    pub fn set_exchange_context(&mut self, context: ExchangeContext) {
        if self.exchange_context.as_ref()!=Some(&context) { self.exchange_context=Some(context); self.projection_source=None; }
    }
    /// Resolve a retained source message/event to its owning virtual item.
    pub fn source_index(&self,key:&str)->Option<usize> {self.projection.iter().position(|item|item.contains_source(key))}
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
        let following=self.list.is_following_tail();
        if self.exchange_context.is_some() && following!=self.exchange_following {
            self.exchange_following=following;
            if following {self.exchange_boundaries.clear();}
            else {self.exchange_boundaries=self.projection.iter().map(ConversationItem::stable_key).collect();}
            self.projection_source=None;
        }
        // Input and decorative invalidations do not change transcript data.
        // Reuse the projection without cloning/grouping all history again.
        if self
            .projection_source
            .as_ref()
            .is_some_and(|source| Rc::ptr_eq(source, &self.rows))
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
        if previous.iter().eq(next.iter()) {
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
            // The preceding row's activity gap depends on its next neighbour.
            if prefix > 0 { self.list.remeasure_items(prefix - 1..prefix); }
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
                if index > 0 { self.list.remeasure_items(index - 1..index); }
            }
        }
        self.projection = Rc::new(next.into_iter().collect());
        self.rebuild_body_slots();
        if !following {
            if let Some(index) = anchor.and_then(|key| {
                self.projection
                    .iter()
                    .position(|item| item.contains_source(&key))
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
        if self.last_compact.iter().eq(latest.iter()) {
            return;
        }
        self.replace_compact(latest.iter().cloned().collect());
        if !self.messages.is_empty() && !self.list.is_following_tail() {
            self.pending_live = Some(PendingLive::Merge(latest));
            return;
        }
        self.merge_latest(latest);
    }
    pub fn release_live(&mut self) {
        let regroup=!self.exchange_boundaries.is_empty();
        self.exchange_boundaries.clear();self.exchange_following=self.list.is_following_tail();
        if regroup {self.projection_source=None;}
        let held = std::mem::take(&mut self.held_body_patches);
        let apply_held = |mut latest: Vec<Rc<Value>>| {
            for message in &mut latest {
                if let Some(body) = held.get(id(message)) {
                    *message = body.clone();
                }
            }
            latest
        };
        let activity_changed = if let Some(activities) = self.pending_activities.take() {
            self.activities = activities;
            true
        } else {
            false
        };
        match self.pending_live.take() {
            Some(PendingLive::Merge(latest)) => self.merge_latest(apply_held(latest)),
            Some(PendingLive::Replace(latest)) => self.update_shared(apply_held(latest)),
            None => {
                if !held.is_empty() {
                    self.merge_latest(held.values().cloned().collect());
                } else if activity_changed {
                    self.update_shared(self.messages.iter().cloned().collect());
                }
            }
        }
        if regroup {let approvals=self.projection_approvals.clone();self.sync_projection(&approvals);}
    }
    pub fn reconcile_activities(&mut self, activities: Vec<Value>, reset: bool) {
        let activities = activities.into_iter().map(Rc::new).collect::<Vec<_>>();
        if reset {
            self.pending_activities = None;
            self.last_activities = activities.clone();
            self.activities = activities;
            self.update_shared(self.messages.iter().cloned().collect());
        } else if self.last_activities != activities {
            self.last_activities = activities.clone();
            if !self.rows.is_empty() && !self.list.is_following_tail() {
                self.pending_activities = Some(activities);
            } else {
                self.pending_activities = None;
                self.activities = activities;
                self.update_shared(self.messages.iter().cloned().collect());
            }
        }
    }
    /// Replace only bodies of existing nonempty streaming assistant messages.
    /// All canonical views advance together; chronology, grouping and list
    /// geometry are preserved until the caller remeasures affected items.
    /// A scrolled-back reader keeps the existing frozen/pending-live path.
    pub fn reconcile_stream_bodies(
        &mut self,
        incoming: &[Value],
        body_available: impl Fn(&str) -> bool,
    ) -> Option<(Vec<Rc<Value>>, Vec<usize>)> {
        if !self.list.is_following_tail()
            || self.has_pending_live()
            || self.loading
            || incoming.len() != self.messages.len()
            || self.last_compact.len() != self.messages.len()
        {
            return None;
        }
        let mut changed = HashMap::new();
        for ((previous, compact), next) in
            self.messages.iter().zip(&self.last_compact).zip(incoming)
        {
            // A page merge may have advanced visible history independently.
            if !Rc::ptr_eq(previous, compact) || id(previous).is_empty() || id(previous) != id(next)
            {
                return None;
            }
            if previous.as_ref() == next {
                continue;
            }
            let (Some(old), Some(new)) = (previous.as_object(), next.as_object()) else {
                return None;
            };
            if old.len() != new.len()
                || old
                    .iter()
                    .any(|(key, value)| key != "text" && new.get(key) != Some(value))
                || previous["role"] != "assistant"
                || next["role"] != "assistant"
                || previous["streamStatus"] != "streaming"
                || next["streamStatus"] != "streaming"
                || previous["text"]
                    .as_str()
                    .is_none_or(|text| text.trim().is_empty())
                || next["text"]
                    .as_str()
                    .is_none_or(|text| text.trim().is_empty())
                || !body_available(id(next))
            {
                return None;
            }
            // Duplicate IDs are not a safe render identity.
            if changed
                .insert(id(next).to_owned(), Rc::new(next.clone()))
                .is_some()
            {
                return None;
            }
        }
        if changed.is_empty() {
            return Some((vec![], vec![]));
        }
        // Share every unchanged JSON payload. No sort, grouping, key generation,
        // full projection equality pass, splice or reader-scroll command runs.
        let mut matches = 0;
        let messages: Vec<_> = self
            .messages
            .iter()
            .map(|message| {
                if let Some(next) = changed.get(id(message)) {
                    matches += 1;
                    next.clone()
                } else {
                    message.clone()
                }
            })
            .collect();
        if matches != changed.len() {
            return None;
        }
        let rows: Vector<_> = self
            .rows
            .iter()
            .map(|row| match row {
                TimelineRow::Message(message) => TimelineRow::Message(
                    changed
                        .get(id(message))
                        .cloned()
                        .unwrap_or_else(|| message.clone()),
                ),
                _ => row.clone(),
            })
            .collect();
        let mut indices = vec![];
        let projection: Vector<_> = self
            .projection
            .iter()
            .enumerate()
            .map(|(index, item)| match item {
                ConversationItem::Message(message) => {
                    if let Some(next) = changed.get(id(message)) {
                        indices.push(index);
                        ConversationItem::Message(next.clone())
                    } else {
                        item.clone()
                    }
                }
                ConversationItem::ExchangeGroup {members,peer_key,exchange_count} => {
                    let mut touched=false;
                    let members=members.iter().map(|member|match member {
                        ConversationItem::Message(message) if changed.contains_key(id(message))=>{touched=true;ConversationItem::Message(changed[id(message)].clone())},
                        _=>member.clone(),
                    }).collect();
                    if touched {indices.push(index);}
                    ConversationItem::ExchangeGroup{members,peer_key:peer_key.clone(),exchange_count:*exchange_count}
                }
                _ => item.clone(),
            })
            .collect();
        let projection_current = self
            .projection_source
            .as_ref()
            .is_some_and(|rows| Rc::ptr_eq(rows, &self.rows));
        self.replace_compact(messages.iter().cloned().collect());
        self.messages = Rc::new(messages.into_iter().collect());
        self.rows = Rc::new(rows.into_iter().collect());
        self.projection = Rc::new(projection);
        if projection_current {
            self.projection_source = Some(self.rows.clone());
        }
        Some((changed.into_values().collect(), indices))
    }
    fn rebuild_body_slots(&mut self) {
        self.body_slots.clear();
        let message_indices: HashMap<_, _> = self
            .messages
            .iter()
            .enumerate()
            .map(|(i, row)| (id(row).to_owned(), i))
            .collect();
        let row_indices: HashMap<_, _> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| match row {
                TimelineRow::Message(row) => Some((id(row).to_owned(), i)),
                _ => None,
            })
            .collect();
        let mut duplicates = HashSet::new();
        let mut seen = HashSet::new();
        for row in self.messages.iter() {
            if !seen.insert(id(row)) {
                duplicates.insert(id(row));
            }
        }
        for (projection_index, item) in self.projection.iter().enumerate() {
            if let ConversationItem::Message(row) = item {
                let key = id(row);
                if !duplicates.contains(key) {
                    if let (Some(message), Some(timeline)) =
                        (message_indices.get(key), row_indices.get(key))
                    {
                        self.body_slots
                            .insert(key.to_owned(), (*message, *timeline, projection_index));
                    }
                }
            }
        }
    }
    fn replace_compact(&mut self, messages: Vector<Rc<Value>>) {
        self.last_compact = messages;
        self.compact_body_slots = None;
        // A full authoritative history supersedes every held body, including
        // a removal or reset that would otherwise resurrect an old message.
        self.held_body_patches.clear();
    }

    /// Coalesce body-only updates while the reader is away from the tail.
    /// The displayed history and its anchor stay frozen. Only the latest body
    /// per ID is retained; no full-history JSON materialization is necessary.
    pub fn defer_stream_body_patches(&mut self, incoming: &[Value]) -> bool {
        if self.list.is_following_tail() || self.messages.is_empty() {
            return false;
        }
        let slots = self.compact_body_slots.get_or_insert_with(|| {
            let mut slots = HashMap::new();
            for (index, row) in self.last_compact.iter().enumerate() {
                slots
                    .entry(id(row).to_owned())
                    .and_modify(|slot| *slot = None)
                    .or_insert(Some(index));
            }
            slots
        });
        let mut changes = Vec::new();
        let mut seen = HashSet::new();
        // Validate the complete batch before advancing even the pending state.
        for next in incoming {
            let key = id(next);
            if key.is_empty() || !seen.insert(key) {
                return false;
            }
            let Some(index) = slots.get(key).copied().flatten() else {
                return false;
            };
            let Some(previous) = self.last_compact.get(index) else {
                return false;
            };
            let (Some(old), Some(new)) = (previous.as_object(), next.as_object()) else {
                return false;
            };
            if old.len() != new.len()
                || old.iter().any(|(key, value)| key != "text" && new.get(key) != Some(value))
                || next["role"] != "assistant"
                || next["streamStatus"] != "streaming"
                || previous["text"].as_str().is_none_or(|text| text.trim().is_empty())
                || next["text"].as_str().is_none_or(|text| text.trim().is_empty())
            {
                return false;
            }
            if previous.as_ref() != next {
                changes.push((index, key.to_owned(), Rc::new(next.clone())));
            }
        }
        for (index, key, body) in changes {
            self.last_compact[index] = body.clone();
            self.held_body_patches.insert(key, body);
        }
        true
    }

    /// Indexed point updates from a publication with authoritative structural
    /// proof. Validate every patch before mutating any retained view.
    pub fn reconcile_stream_body_patches(
        &mut self,
        incoming: &[Value],
        body_available: impl Fn(&str) -> bool,
    ) -> Option<(Vec<Rc<Value>>, Vec<usize>)> {
        if !self.list.is_following_tail() || self.has_pending_live() || self.loading {
            return None;
        }
        let mut patches = Vec::new();
        let mut seen = HashSet::new();
        for next in incoming {
            let key = id(next);
            if !seen.insert(key) || !body_available(key) {
                return None;
            }
            let &(message, timeline, projection) = self.body_slots.get(key)?;
            let previous = self.messages.get(message)?;
            if !Rc::ptr_eq(previous, self.last_compact.get(message)?) {
                return None;
            }
            let (old, new) = (previous.as_object()?, next.as_object()?);
            if old.len() != new.len()
                || old
                    .iter()
                    .any(|(key, value)| key != "text" && new.get(key) != Some(value))
                || next["role"] != "assistant"
                || next["streamStatus"] != "streaming"
                || previous["text"].as_str()?.trim().is_empty()
                || next["text"].as_str()?.trim().is_empty()
            {
                return None;
            }
            if previous.as_ref() != next {
                patches.push((message, timeline, projection, Rc::new(next.clone())));
            }
        }
        // Retained readers keep immutable roots. Persistent vectors copy only
        // the affected tree blocks, even when preparation keys and row closures
        // still own the prior outer Rc.
        let current = self
            .projection_source
            .as_ref()
            .is_some_and(|source| Rc::ptr_eq(source, &self.rows));
        if current {
            self.projection_source = None;
        }
        let mut bodies = Vec::new();
        let mut indices = Vec::new();
        for (message, timeline, projection, next) in patches {
            Rc::make_mut(&mut self.messages)[message] = next.clone();
            self.last_compact[message] = next.clone();
            Rc::make_mut(&mut self.rows)[timeline] = TimelineRow::Message(next.clone());
            Rc::make_mut(&mut self.projection)[projection] =
                ConversationItem::Message(next.clone());
            bodies.push(next);
            indices.push(projection);
        }
        if current {
            self.projection_source = Some(self.rows.clone());
        }
        Some((bodies, indices))
    }
    /// Reconcile the native cache's entire known task history, including loaded
    /// pages. Unlike a compact tail, this list is authoritative for removals.
    /// A journal reset invalidates page cursors and any held prior revision.
    pub fn reconcile(&mut self, messages: Vec<Value>, reset: bool) {
        let messages = messages.into_iter().map(Rc::new).collect::<Vec<_>>();
        if reset {
            self.exchange_boundaries.clear();self.exchange_following=self.list.is_following_tail();
            self.projection_source=None;
            self.boundary = HistoryBoundary::Unknown;
            self.loading = false;
            self.error = None;
            self.pending_live = None;
            self.pending_activities = None;
            self.activities.clear();
            self.last_activities.clear();
            self.replace_compact(messages.iter().cloned().collect());
            self.update_shared(messages);
        } else if self.last_compact.iter().ne(messages.iter()) {
            if let HistoryBoundary::More(cursor) = &self.boundary {
                if !messages.iter().any(|message| id(message) == cursor) {
                    self.boundary = messages
                        .first()
                        .map_or(HistoryBoundary::Unknown, |message| {
                            HistoryBoundary::More(id(message).to_owned())
                        });
                }
            }
            self.replace_compact(messages.iter().cloned().collect());
            if !self.messages.is_empty() && !self.list.is_following_tail() {
                self.pending_live = Some(PendingLive::Replace(messages));
            } else {
                self.pending_live = None;
                self.update_shared(messages);
            }
        }
    }
    fn merge_latest(&mut self, latest: Vec<Rc<Value>>) {
        let mut merged = self.messages.iter().cloned().collect::<Vec<_>>();
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
    /// Retire unconfirmed presentation rows and install the first real page in
    /// one projection update. Held updates must never replay cache-only IDs.
    pub(super) fn merge_startup_page(&mut self, page: &Value, startup_ids: &HashSet<String>) {
        if !startup_ids.is_empty() {
            self.messages = Rc::new(self.messages.iter()
                .filter(|row| !startup_ids.contains(id(row))).cloned().collect());
            self.last_compact = self.last_compact.iter()
                .filter(|row| !startup_ids.contains(id(row))).cloned().collect();
            self.compact_body_slots = None;
            self.held_body_patches.retain(|key, _| !startup_ids.contains(key));
            match &mut self.pending_live {
                Some(PendingLive::Replace(rows) | PendingLive::Merge(rows)) => {
                    rows.retain(|row| !startup_ids.contains(id(row)));
                }
                None => {}
            }
            // Keep the previous projection until merge_page rebuilds once.
            // That preserves off-tail anchors whose IDs occur in the real page.
            self.projection_source = None;
        }
        self.merge_page(page);
        if !startup_ids.is_empty() { self.rebuild_body_slots(); }
    }
    pub fn merge_page(&mut self, page: &Value) {
        let mut merged: Vec<_> = page["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .cloned()
            .map(Rc::new)
            .collect();
        let ids: HashSet<String> = merged
            .iter()
            .map(|message| id(message).to_owned())
            .collect();
        // The caller has already accepted this page at the current revision.
        // An overlapping row therefore supersedes an earlier held chunk.
        // Rebase a held replacement too, so returning live cannot discard a
        // page loaded while the reader was detached from the tail.
        self.held_body_patches.retain(|key, _| !ids.contains(key));
        let accepted: HashMap<_, _> = merged
            .iter()
            .map(|row| (id(row).to_owned(), row.clone()))
            .collect();
        let rebase = |rows: &mut Vec<Rc<Value>>| {
            for row in rows {
                if let Some(current) = accepted.get(id(row)) {
                    *row = current.clone();
                }
            }
        };
        match &mut self.pending_live {
            Some(PendingLive::Replace(latest)) => {
                let mut paged = merged.clone();
                paged.extend(latest.iter().filter(|row| !ids.contains(id(row))).cloned());
                *latest = paged;
            }
            Some(PendingLive::Merge(latest)) => rebase(latest),
            None => {}
        }
        self.compact_body_slots = None;
        for row in self.last_compact.iter_mut() {
            if let Some(current) = accepted.get(id(row)) {
                *row = current.clone();
            }
        }
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
        self.messages = Rc::new(messages.into_iter().collect());
        self.rows = Rc::new(rows.into_iter().collect());
        self.sync_projection(&[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    // Allow the grouping projection to be referenced from test code without
    // pulling the public api into the renderer's call path.

    fn streamed(id: &str, text: &str, at: i64) -> Value {
        json!({"id":id,"taskId":"task","role":"assistant","streamStatus":"streaming","createdAt":at,"text":text})
    }

    #[::core::prelude::v1::test]
    fn held_stream_chunks_preserve_reader_and_coalesce_latest_body() {
        let mut state = TranscriptState::default();
        state.reconcile((0..20_000).map(|n| streamed(&format!("m{n}"), "initial", n)).collect(), true);
        state.list.set_follow_mode(FollowMode::Normal);
        state.list.scroll_to(ListOffset { item_ix: 120, offset_in_item: px(7.) });
        let anchor = state.list.logical_scroll_top();
        let messages = state.messages.clone();
        let rows = state.rows.clone();
        let projection = state.projection.clone();
        for chunk in 0..256 {
            assert!(state.defer_stream_body_patches(&[streamed("m19999", &format!("chunk {chunk}"), 19_999)]));
        }
        assert!(Rc::ptr_eq(&state.messages, &messages));
        assert!(Rc::ptr_eq(&state.rows, &rows));
        assert!(Rc::ptr_eq(&state.projection, &projection));
        assert_eq!(state.list.logical_scroll_top().item_ix, anchor.item_ix);
        assert_eq!(state.list.logical_scroll_top().offset_in_item, anchor.offset_in_item);
        assert_eq!(state.held_body_patches.len(), 1);
        assert_eq!(state.messages[19_999]["text"], "initial");
        assert!(state.has_pending_live());
        state.release_live();
        assert!(!state.has_pending_live());
        assert_eq!(state.messages[19_999]["text"], "chunk 255");
        assert!(Rc::ptr_eq(&state.messages[19_998], &messages[19_998]));
    }

    #[::core::prelude::v1::test]
    fn held_stream_updates_pending_new_message_and_releases_activity_together() {
        let mut state = TranscriptState::default();
        state.reconcile(vec![streamed("old", "old body", 1)], true);
        state.list.set_follow_mode(FollowMode::Normal);
        state.reconcile(vec![streamed("old", "old body", 1), streamed("new", "first", 3)], false);
        state.reconcile_activities(vec![json!({"id":"event","createdAt":2})], false);
        assert!(state.defer_stream_body_patches(&[streamed("new", "latest", 3)]));
        assert_eq!(state.messages.len(), 1);
        state.release_live();
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1]["text"], "latest");
        assert_eq!(state.rows.iter().map(TimelineRow::key).collect::<Vec<_>>(), ["message:old", "event:event", "message:new"]);
    }

    #[::core::prelude::v1::test]
    fn held_stream_preserves_page_and_cursor_and_authoritative_removal_wins() {
        let mut state = TranscriptState::default();
        state.reconcile(vec![streamed("live", "initial", 5)], true);
        state.list.set_follow_mode(FollowMode::Normal);
        state.loading = true;
        assert!(state.defer_stream_body_patches(&[streamed("live", "held", 5)]));
        state.merge_page(&json!({"messages":[streamed("older", "older page", 1)],"nextBeforeId":"older"}));
        state.loading = false;
        state.release_live();
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[0]["id"], "older");
        assert_eq!(state.messages[1]["text"], "held");
        assert_eq!(state.before_id().as_deref(), Some("older"));
        assert!(state.defer_stream_body_patches(&[streamed("live", "obsolete", 5)]));
        state.reconcile(vec![streamed("older", "older page", 1)], false);
        state.release_live();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0]["id"], "older");
        state.reconcile(vec![streamed("live", "new epoch", 9)], true);
        state.release_live();
        assert!(!state.has_pending_live());
        assert_eq!(state.messages[0]["text"], "new epoch");
    }

    #[::core::prelude::v1::test]
    fn held_stream_batch_rejects_structure_atomically_and_reset_discards_it() {
        let mut state = TranscriptState::default();
        state.reconcile(vec![streamed("a", "initial a", 1), streamed("b", "initial b", 2)], true);
        state.list.set_follow_mode(FollowMode::Normal);
        let mut invalid = streamed("b", "changed b", 2);
        invalid["streamStatus"] = json!("complete");
        assert!(!state.defer_stream_body_patches(&[streamed("a", "changed a", 1), invalid]));
        assert!(!state.has_pending_live());
        assert_eq!(state.last_compact[0]["text"], "initial a");
        assert!(!state.defer_stream_body_patches(&[streamed("a", "duplicate 1", 1), streamed("a", "duplicate 2", 1)]));
        assert!(state.defer_stream_body_patches(&[streamed("a", "held", 1)]));
        state.reconcile(vec![streamed("a", "authoritative reset", 4)], true);
        state.release_live();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0]["text"], "authoritative reset");
        assert!(!state.has_pending_live());
    }

    #[::core::prelude::v1::test]
    fn accepted_page_supersedes_overlapping_held_body_and_survives_pending_replace() {
        let mut state = TranscriptState::default();
        state.reconcile(vec![streamed("live", "initial", 5)], true);
        state.list.set_follow_mode(FollowMode::Normal);
        state.reconcile(vec![streamed("live", "new structure", 5), streamed("next", "new message", 7)], false);
        assert!(state.defer_stream_body_patches(&[streamed("live", "held before page", 5)]));
        state.merge_page(&json!({"messages":[streamed("older", "page", 1), streamed("live", "page is newer", 5)],"nextBeforeId":"older"}));
        state.release_live();
        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.messages[0]["id"], "older");
        assert_eq!(state.messages[1]["text"], "page is newer");
        assert_eq!(state.messages[2]["id"], "next");
        assert_eq!(state.before_id().as_deref(), Some("older"));
        assert!(!state.has_pending_live());
    }

    #[::core::prelude::v1::test]
    fn retained_owners_at_100k_rows_keep_point_updates_bounded() {
        use std::cell::Cell;
        #[derive(Debug)]
        struct Probe(Rc<Cell<usize>>, usize);
        impl Clone for Probe {
            fn clone(&self) -> Self {
                self.0.set(self.0.get() + 1);
                Self(self.0.clone(), self.1)
            }
        }
        let clones = Rc::new(Cell::new(0));
        let mut rows = Rc::new(
            (0..100_000)
                .map(|i| Probe(clones.clone(), i))
                .collect::<Vector<_>>(),
        );
        let retained = rows.clone();
        Rc::make_mut(&mut rows)[50_000].1 = 999_999;
        assert!(
            clones.get() < 256,
            "persistent point replacement cloned {} records",
            clones.get()
        );
        assert_eq!(retained[50_000].1, 50_000);
        assert_eq!(rows[50_000].1, 999_999);

        let mut state = TranscriptState::default();
        state.reconcile((0..100_000).map(|i| json!({"id":format!("m{i}"),"role":"assistant","streamStatus":"streaming","createdAt":i,"text":"before"})).collect(), true);
        let messages = state.messages.clone();
        let timeline = state.rows.clone();
        let projection = state.projection.clone();
        state.reconcile_stream_body_patches(&[json!({"id":"m50000","role":"assistant","streamStatus":"streaming","createdAt":50000,"text":"after"})], |_| true).unwrap();
        assert_eq!(messages[50_000]["text"], "before");
        assert_eq!(state.messages[50_000]["text"], "after");
        assert!(Rc::ptr_eq(&messages[49_999], &state.messages[49_999]));
        assert!(
            matches!((&timeline[50_000], &state.rows[50_000]), (TimelineRow::Message(old),TimelineRow::Message(new)) if old["text"] == "before" && new["text"] == "after")
        );
        assert!(
            matches!((&projection[50_000], &state.projection[50_000]), (ConversationItem::Message(old),ConversationItem::Message(new)) if old["text"] == "before" && new["text"] == "after")
        );
    }
    #[::core::prelude::v1::test]
    fn indexed_body_patch_preserves_unchanged_payload_and_rejects_structural_change() {
        let mut state = TranscriptState::default();
        let row = |id: &str, text: &str| json!({"id":id,"role":"assistant","streamStatus":"streaming","createdAt":0,"text":text});
        state.reconcile(vec![row("a", "old"), row("b", "unchanged")], true);
        let unchanged = state.messages[1].clone();
        let (bodies, slots) = state
            .reconcile_stream_body_patches(&[row("a", "new")], |_| true)
            .unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(slots, vec![0]);
        assert!(Rc::ptr_eq(&state.messages[1], &unchanged));
        assert_eq!(state.messages[0]["text"], "new");
        let mut bad = row("a", "bad");
        bad["createdAt"] = json!(2);
        assert!(
            state
                .reconcile_stream_body_patches(&[bad], |_| true)
                .is_none()
        );
        assert_eq!(state.messages[0]["text"], "new");
    }
    #[::core::prelude::v1::test]
    fn streaming_body_shortcut_rejects_structure_metadata_and_empty_transitions() {
        let original = json!({"id":"stream", "taskId":"task", "role":"assistant", "text":"initial body",
            "streamStatus":"streaming", "createdAt":1, "attachments":[], "responseMetadata":null});
        let mut state = TranscriptState::default();
        state.reconcile(vec![original.clone()], true);
        let initial_rows = state.rows.clone();
        let mut body = original.clone();
        body["text"] = json!("new streamed body");
        assert!(
            state
                .reconcile_stream_bodies(&[body.clone()], |_| false)
                .is_none(),
            "unprepared bodies require normal preparation"
        );
        for (key, value) in [
            ("streamStatus", json!("complete")),
            ("attachments", json!([{"id":"attachment"}])),
            ("responseMetadata", json!({"model":"new-model"})),
            ("phase", json!("final_answer")),
            ("createdAt", json!(2)),
            ("role", json!("user")),
            ("text", json!("")),
            ("text", json!("   ")),
            ("id", json!("replacement")),
        ] {
            let mut rejected = body.clone();
            rejected[key] = value;
            assert!(
                state
                    .reconcile_stream_bodies(&[rejected], |_| true)
                    .is_none(),
                "{key} must use structural reconciliation"
            );
            assert!(
                Rc::ptr_eq(&state.rows, &initial_rows),
                "failed proof cannot mutate canonical state"
            );
        }
        assert!(state.reconcile_stream_bodies(&[], |_| true).is_none());
        assert!(
            state
                .reconcile_stream_bodies(&[body.clone(), body.clone()], |_| true)
                .is_none()
        );
        let (changed, indices) = state
            .reconcile_stream_bodies(&[body.clone()], |_| true)
            .unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(indices, vec![0]);
        assert_eq!(state.messages[0].as_ref(), &body);
        let rows = state.rows.clone();
        let projection = state.projection.clone();
        state.sync_projection(&[]);
        assert!(Rc::ptr_eq(&state.rows, &rows));
        assert!(
            Rc::ptr_eq(&state.projection, &projection),
            "no redundant projection after body substitution"
        );
        assert!(
            state
                .reconcile_stream_bodies(&[body], |_| true)
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[::core::prelude::v1::test]
    fn inactive_tail_eviction_restores_cursor_and_protects_held_reader() {
        let mut state = TranscriptState::default();
        let messages = (0..100)
            .map(|n| json!({"id":format!("m{n}"),"role":"user","createdAt":n,"text":"history"}))
            .collect();
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
        state.update(vec![
            json!({"id":"m", "role":"user", "text":"before", "createdAt":1}),
        ]);
        let initial = state.projection.clone();
        for _ in 0..100 {
            state.sync_projection(&[]);
        }
        assert!(Rc::ptr_eq(&initial, &state.projection));
        state.update(vec![
            json!({"id":"m", "role":"user", "text":"after", "createdAt":1}),
        ]);
        assert!(!Rc::ptr_eq(&initial, &state.projection));
        assert!(
            matches!(&state.projection[0], ConversationItem::Message(value) if value["text"] == "after")
        );
        let approval = ApprovalRow {
            id: "approval".into(),
            task_id: "task".into(),
            status: "approved".into(),
            created_at: 2,
            resolved_at: Some(3),
            rule_id: None,
            summary: "Allowed".into(),
            tool: "exec".into(),
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
        assert_eq!(
            state.messages.iter().cloned().collect::<Vec<_>>(),
            vec![Rc::new(json!({"id":"live","text":"updated"}))]
        );
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
        let first = state.projection.front().unwrap();
        assert_eq!(first.stable_key(), "reasoning-group:r1");
        assert_eq!(first.row_count(), 2);
        assert_eq!(state.projection.len(), 1);

        state.reconcile_activities(vec![event("r1", 1), event("r2", 2), event("r3", 3)], false);
        let after = state.projection.front().unwrap();
        assert_eq!(after.stable_key(), "reasoning-group:r1");
        assert_eq!(after.row_count(), 3);
        assert_eq!(state.projection.len(), 1);
    }
}

#[cfg(test)]
mod exchange_projection_tests {
    use super::*; use serde_json::json;
    fn msg(id:&str,at:i64)->Value{json!({"id":id,"taskId":"here","role":"user","senderAgentId":"peer-agent","collaborationId":id,"createdAt":at,"text":id})}
    #[::core::prelude::v1::test]
    fn exchange_group_append_and_expanded_prepend_preserve_member_boundaries() {
        let base=1_760_000_000_000;
        let mut state=TranscriptState::default();
        state.set_exchange_context(ExchangeContext::from_snapshot(&json!({"collaborations":(["older","one","two","three"].map(|id|json!({"id":id,"fromTaskId":"peer","fromAgentId":"peer-agent","toTaskId":"here"})))}),"here"));
        state.update(vec![msg("one",base),msg("two",base+1_000)]);state.sync_projection(&[]);
        assert_eq!(state.list.item_count(),1);assert_eq!(state.source_index("message:two"),Some(0));
        let key=state.projection[0].stable_key();let held=state.projection.clone();
        state.update(vec![msg("one",base),msg("two",base+1_000),msg("three",base+2_000)]);state.sync_projection(&[]);
        assert_eq!(state.projection[0].stable_key(),key);assert_eq!(state.list.item_count(),1);assert_eq!(held[0].row_count(),2);assert_eq!(state.projection[0].row_count(),3);
        // This offset represents a reader inside the already expanded diagram.
        let expanded_group=state.projection[0].clone();
        state.list.set_follow_mode(FollowMode::Normal);state.list.scroll_to(ListOffset{item_ix:0,offset_in_item:px(75.)});
        state.merge_page(&json!({"messages":[msg("older",base-1_000)],"hasMore":false}));state.sync_projection(&[]);
        assert_eq!(state.source_index(&key),Some(1));assert_eq!(state.source_index("message:three"),Some(1));assert_eq!(state.list.item_count(),2);
        assert_eq!(state.projection[1].stable_key(),key);assert_eq!(state.projection[1].row_count(),3);
        assert_eq!(state.list.logical_scroll_top().item_ix,1);assert_eq!(state.list.logical_scroll_top().offset_in_item,px(75.));
        assert_eq!(state.projection[1],expanded_group,"prepended rows cannot alter the retained expanded group's members, bodies or extent");
        state.release_live();state.list.set_follow_mode(FollowMode::Tail);state.sync_projection(&[]);
        assert_eq!(state.list.item_count(),1);assert_eq!(state.source_index(&key),Some(0));
    }
}
