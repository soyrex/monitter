//! Native channel UI over the existing service command contract.
//! Every recipient/member/message is read from the real backend. All service
//! calls execute on background workers; this module adds no command authority.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::{
        Disableable,
        button::*,
        input::{Input, InputState, Textarea, TextareaState},
        text::TextView,
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::{ChannelMessagesPage, DesktopService};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn rows(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
fn borrowed_rows<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}
fn visible_channels(snapshot: &Value) -> impl Iterator<Item = &Value> {
    borrowed_rows(snapshot, "channels").iter().filter(|channel| {
        !channel["id"].as_str().unwrap_or_default().starts_with("__monitter-project-board__:")
    })
}
fn channel_metadata_changed(previous: &Value, snapshot: &Value) -> bool {
    let mut previous = visible_channels(previous);
    let mut current = visible_channels(snapshot);
    loop {
        match (previous.next(), current.next()) {
            (None, None) => return false,
            (Some(previous), Some(current)) => {
                fn metadata(channel: &Value) -> impl Iterator<Item = (&String, &Value)> {
                    channel.as_object().into_iter().flat_map(|object| object.iter()).filter(|(key, _)| key.as_str() != "messages")
                }
                if !metadata(previous).eq(metadata(current)) { return true; }
            }
            _ => return true,
        }
    }
}
fn channel_agents_changed(previous: &Value, snapshot: &Value) -> bool {
    fn metadata(agent: &Value) -> (&Value, &Value, &Value) { (&agent["id"], &agent["name"], &agent["internal"]) }
    // Compare the fields rendered in the member editor and author fallback;
    // agent runtime/task changes do not alter this view.
    !borrowed_rows(previous, "agents").iter().map(metadata)
        .eq(borrowed_rows(snapshot, "agents").iter().map(metadata))
}
fn channel_messages_changed(previous: Option<&Value>, channel: &Value, reset: bool) -> bool {
    reset || previous.is_none_or(|previous| borrowed_rows(previous, "messages") != borrowed_rows(channel, "messages"))
}
fn member(channel: &Value, id: &str) -> bool {
    borrowed_rows(channel, "agentIds").iter().any(|value| value.as_str() == Some(id))
}
/// Root-owned paging boundaries are invalidated by resets or cursor deletion;
/// missing means unknown, while an explicit None means a successfully loaded end.
pub(super) fn reconcile_cursors(
    cursors: &mut HashMap<String, Option<String>>,
    snapshot: &Value,
    full_reset: bool,
    resets: &[String],
) {
    if full_reset {
        cursors.clear();
        return;
    }
    for id in resets {
        cursors.remove(id);
    }
    let channels = borrowed_rows(snapshot, "channels");
    cursors.retain(|id, cursor| {
        channels
            .iter()
            .find(|row| row["id"].as_str() == Some(id.as_str()))
            .is_some_and(|row| {
                cursor.as_ref().is_none_or(|cursor| {
                    borrowed_rows(row, "messages")
                        .iter()
                        .any(|message| message["id"].as_str() == Some(cursor.as_str()))
                })
            })
    });
}
enum ResultKind {
    Snapshot,
    Created(HashSet<String>),
    Sent { channel: String, draft: String },
}

#[derive(Clone)]
pub enum ChannelEvent {
    Refresh,
    HistoryPage {
        channel: String,
        before: Option<String>,
        page: Result<ChannelMessagesPage, String>,
    },
}
impl EventEmitter<ChannelEvent> for ChannelPane {}
pub struct ChannelPane {
    service: DesktopService,
    snapshot: Rc<Value>,
    revision: String,
    selected: Option<String>,
    creating: bool,
    name: Entity<InputState>,
    topic: Entity<InputState>,
    turn_limit: Entity<InputState>,
    composer: Entity<TextareaState>,
    recipients: HashSet<String>,
    recipient_selections: HashMap<String, HashSet<String>>,
    drafts: HashMap<String, String>,
    selection_removed: bool,
    transcripts: HashMap<String, crate::transcript::TranscriptState>,
    next_before: HashMap<String, Option<String>>,
    clear_sent: Option<(String, String)>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
}
impl ChannelPane {
    pub fn new(service: DesktopService, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            service,
            snapshot: Rc::new(Value::Null),
            revision: String::new(),
            selected: None,
            creating: false,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Channel name")),
            topic: cx.new(|cx| InputState::new(window, cx).placeholder("Topic")),
            turn_limit: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Conversation turn limit (1–20)")
            }),
            composer: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("Message selected recipients…")
            }),
            recipients: HashSet::new(),
            recipient_selections: HashMap::new(),
            drafts: HashMap::new(),
            selection_removed: false,
            transcripts: HashMap::new(),
            next_before: HashMap::new(),
            clear_sent: None,
            busy: false,
            error: None,
            notice: None,
        }
    }
    /// Host updates this entity after each real backend snapshot change.
    pub fn sync(
        &mut self,
        snapshot: Rc<Value>,
        revision: &str,
        reset: bool,
        reset_channels: &[String],
        cursors: &HashMap<String, Option<String>>,
        cx: &mut Context<Self>,
    ) {
        // Conservative adapter for direct snapshots/fixtures lacking a delta.
        let previous: HashMap<&str, &Value> = visible_channels(&self.snapshot)
            .filter_map(|channel| Some((channel["id"].as_str()?, channel))).collect();
        let changed_channels = visible_channels(&snapshot).filter_map(|channel| {
            let id = channel["id"].as_str()?;
            channel_messages_changed(previous.get(id).copied(), channel, false).then(|| id.to_owned())
        }).collect();
        self.sync_scoped(snapshot, revision, reset, reset_channels, cursors, &changed_channels, cx);
    }
    /// Production delta boundary: channel body work is restricted to changed IDs.
    pub fn sync_scoped(
        &mut self,
        snapshot: Rc<Value>,
        revision: &str,
        reset: bool,
        reset_channels: &[String],
        cursors: &HashMap<String, Option<String>>,
        changed_channels: &HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        let channels_changed = channel_metadata_changed(&self.snapshot, &snapshot);
        let body_changes = visible_channels(&snapshot).any(|channel| channel["id"].as_str().is_some_and(|id| changed_channels.contains(id)));
        let metadata_changed = channel_agents_changed(&self.snapshot, &snapshot)
            || self.snapshot["settings"] != snapshot["settings"];
        let cursors_changed = self.next_before != *cursors;
        let previous = std::mem::replace(&mut self.snapshot, snapshot);
        self.revision = revision.to_owned();
        if cursors_changed { self.next_before = cursors.clone(); }
        if !channels_changed && !body_changes && !reset && reset_channels.is_empty() {
            if metadata_changed || cursors_changed { cx.notify(); }
            return;
        }
        let channels: Vec<&Value> = visible_channels(&self.snapshot).collect();
        let ids: HashSet<&str> = channels.iter().filter_map(|row| row["id"].as_str()).collect();
        self.transcripts.retain(|id, _| ids.contains(id.as_str()));
        self.recipient_selections.retain(|id, recipients| {
            if let Some(channel) = channels.iter().find(|row| row["id"].as_str() == Some(id.as_str())) {
                recipients.retain(|recipient| member(channel, recipient));
                true
            } else { false }
        });
        if self.selected.as_ref().is_some_and(|id| !ids.contains(id.as_str())) {
            self.selection_removed = true;
        }
        let previous_channels: HashMap<&str, &Value> = visible_channels(&previous)
            .filter_map(|channel| Some((channel["id"].as_str()?, channel))).collect();
        for channel in &channels {
            let id = field(channel, "id");
            let old = previous_channels.get(id.as_str()).copied();
            let reset = reset || reset_channels.contains(&id);
            let missing = !self.transcripts.contains_key(&id);
            if missing || reset || (changed_channels.contains(&id) && channel_messages_changed(old, channel, false)) {
                self.transcripts.entry(id).or_default().reconcile(rows(channel, "messages"), reset);
            }
        }
        if let Some(channel) = channels.iter().find(|channel| channel["id"].as_str() == self.selected.as_deref()) {
            self.recipients.retain(|id| member(channel, id));
        }
        cx.notify();
    }
    pub fn accept_history_page(
        &mut self,
        channel: &str,
        result: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok(page) => {
                if let Some(state) = self.transcripts.get_mut(channel) {
                    state.merge_page(&page);
                }
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
    fn channels(&self) -> Vec<Value> {
        rows(&self.snapshot, "channels")
            .into_iter()
            .filter(|channel| !field(channel, "id").starts_with("__monitter-project-board__:"))
            .collect()
    }
    fn current(&self) -> Option<Value> {
        self.channels()
            .into_iter()
            .find(|channel| Some(field(channel, "id")) == self.selected)
    }
    pub(super) fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = &self.selected {
            self.drafts
                .insert(previous.clone(), self.composer.read(cx).value().to_string());
            self.recipient_selections
                .insert(previous.clone(), self.recipients.clone());
        }
        self.selected = id;
        self.creating = false;
        self.recipients = self
            .selected
            .as_ref()
            .and_then(|id| self.recipient_selections.get(id))
            .cloned()
            .unwrap_or_default();
        self.error = None;
        self.notice = None;
        let channel = self.current().unwrap_or(Value::Null);
        let name = field(&channel, "name");
        let topic = field(&channel, "description");
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.topic
            .update(cx, |input, cx| input.set_value(topic, window, cx));
        let limit = channel["agentConversationTurnLimit"]
            .as_u64()
            .unwrap_or(6)
            .to_string();
        self.turn_limit
            .update(cx, |input, cx| input.set_value(limit, window, cx));
        let draft = self
            .selected
            .as_ref()
            .and_then(|id| self.drafts.get(id))
            .cloned()
            .unwrap_or_default();
        self.composer
            .update(cx, |input, cx| input.set_value(draft, window, cx));
        cx.notify();
    }
    pub(super) fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select(None, window, cx);
        self.creating = true;
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let mut channel = if self.creating {
            json!({"id":"","name":"","description":"","agentIds":[],"messages":[]})
        } else {
            let Some(channel) = self.current() else {
                return;
            };
            channel
        };
        channel["name"] = json!(self.name.read(cx).value().to_string());
        channel["description"] = json!(self.topic.read(cx).value().to_string());
        let kind = if self.creating {
            ResultKind::Created(
                self.channels()
                    .iter()
                    .map(|channel| field(channel, "id"))
                    .collect(),
            )
        } else {
            ResultKind::Snapshot
        };
        self.command("save_channel", json!({"channel":channel}), kind, cx);
    }
    fn command(
        &mut self,
        command: &'static str,
        args: Value,
        kind: ResultKind,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.notice = None;
        let service = self.service.clone();
        let worker = cx
            .background_executor()
            .spawn(async move { service.invoke(command, args) });
        cx.spawn(async move |pane, cx| {
            let result = worker.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.busy = false;
                match result {
                    Err(error) => pane.error = Some(error),
                    Ok(value) => match kind {
                        ResultKind::Created(old_ids) => {
                            if let Some(channel) = rows(&value, "channels")
                                .into_iter()
                                .find(|channel| !old_ids.contains(&field(channel, "id")))
                            {
                                let id = field(&channel, "id");
                                pane.selected = Some(id.clone());
                                pane.creating = false;
                                pane.recipients = pane
                                    .recipient_selections
                                    .get(&id)
                                    .cloned()
                                    .unwrap_or_default();
                            }
                            pane.notice = Some("Channel saved.".into());
                        }
                        ResultKind::Sent { channel, draft } => {
                            pane.clear_sent = Some((channel, draft));
                            pane.notice = Some("Channel message accepted.".into());
                        }
                        ResultKind::Snapshot => {
                            pane.notice = Some("Channel updated.".into());
                        }
                    },
                }
                if let Some(channel) = pane.current() {
                    pane.recipients.retain(|id| member(&channel, id));
                }
                if let Some(channel) = pane.current() {
                    pane.recipient_selections
                        .insert(field(&channel, "id"), pane.recipients.clone());
                }
                cx.emit(ChannelEvent::Refresh);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn send(&mut self, cx: &mut Context<Self>) {
        let Some(channel) = self.current() else {
            return;
        };
        let id = field(&channel, "id");
        let draft = self.composer.read(cx).value().to_string();
        let mut ids: Vec<_> = self
            .recipients
            .iter()
            .filter(|id| member(&channel, id))
            .cloned()
            .collect();
        ids.sort();
        if draft.trim().is_empty() || ids.is_empty() {
            self.error = Some("Choose at least one channel member and enter a message.".into());
            cx.notify();
            return;
        }
        self.command(
            "send_channel_message",
            json!({"channelId":id,"text":draft,"agentIds":ids,"attachmentIds":[]}),
            ResultKind::Sent { channel: id, draft },
            cx,
        );
    }
    fn configure_conversation(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Ok(limit) = self.turn_limit.read(cx).value().trim().parse::<u32>() else {
            self.error = Some("Enter a conversation turn limit from 1 to 20.".into());
            cx.notify();
            return;
        };
        if !(1..=20).contains(&limit) {
            self.error = Some("Conversation turn limit must be between 1 and 20.".into());
            cx.notify();
            return;
        }
        self.command(
            "set_channel_agent_conversation",
            json!({"channelId":id,"enabled":enabled,"turnLimit":limit}),
            ResultKind::Snapshot,
            cx,
        );
    }
    fn load_earlier(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(channel) = self.current() else {
            return;
        };
        let id = field(&channel, "id");
        if self.next_before.get(&id) == Some(&None) {
            return;
        }
        let before = self.next_before.get(&id).cloned().flatten().or_else(|| {
            rows(&channel, "messages")
                .first()
                .map(|message| field(message, "id"))
        });
        self.busy = true;
        self.error = None;
        let service = self.service.clone();
        let request_id = id.clone();
        let request_before = before.clone();
        let task = cx.background_executor().spawn(async move {
            service.read_channel_messages(&request_id, request_before.as_deref(), Some(64))
        });
        cx.spawn(async move |pane, cx| {
            let page = task.await;
            let _ = pane.update(cx, |_, cx| {
                cx.emit(ChannelEvent::HistoryPage {
                    channel: id,
                    before,
                    page,
                })
            });
        })
        .detach();
        cx.notify();
    }
    fn render_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let channel = self.current();
        let mut panel = div()
            .flex()
            .flex_col()
            .size_full()
            .gap_2()
            .min_w(px(0.))
            .child(Input::new(&self.name))
            .child(Input::new(&self.topic))
            .child(
                Button::new("channel-save")
                    .label(if self.creating {
                        "Create channel"
                    } else {
                        "Save name and topic"
                    })
                    .disabled(self.busy)
                    .on_click(cx.listener(|pane, _, _, cx| pane.save(cx))),
            );
        let Some(channel) = channel else {
            return panel
                .child(if self.creating {
                    "Create the channel, then add members and choose recipients."
                } else {
                    "Select a channel or create one."
                })
                .into_any_element();
        };
        let channel_id = field(&channel, "id");
        let mut agents = div()
            .id("channel-member-editor")
            .max_h(px(175.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1()
            .child("Membership and recipients");
        for agent in rows(&self.snapshot, "agents")
            .into_iter()
            .filter(|agent| !agent["internal"].as_bool().unwrap_or(false))
        {
            let agent_id = field(&agent, "id");
            let name = field(&agent, "name");
            let joined = member(&channel, &agent_id);
            let membership_channel = channel_id.clone();
            let membership_agent = agent_id.clone();
            let mut row=div().flex().gap_2().items_center()
                .child(Button::new(SharedString::from(format!("channel-member-{agent_id}")))
                    .label(format!("{} {}",if joined{"Remove"}else{"Add"},name)).disabled(self.busy)
                    .on_click(cx.listener(move |pane,_,_,cx|pane.command("set_channel_membership",json!({"channelId":membership_channel,"agentId":membership_agent,"member":!joined}),ResultKind::Snapshot,cx))));
            if joined {
                let receiving = self.recipients.contains(&agent_id);
                row = row.child(
                    Button::new(SharedString::from(format!("channel-recipient-{agent_id}")))
                        .label(if receiving {
                            "Recipient selected"
                        } else {
                            "Select recipient"
                        })
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            if !pane.recipients.remove(&agent_id) {
                                pane.recipients.insert(agent_id.clone());
                            }
                            cx.notify();
                        })),
                );
            }
            agents = agents.child(row);
        }
        panel = panel.child(agents);
        let enabled = channel["agentConversationEnabled"]
            .as_bool()
            .unwrap_or(false);
        let paused = channel["agentConversationPaused"]
            .as_bool()
            .unwrap_or(false);
        let used = channel["agentConversationTurnsUsed"]
            .as_u64()
            .unwrap_or_default();
        let limit = channel["agentConversationTurnLimit"].as_u64().unwrap_or(6);
        panel =
            panel
                .child(format!(
                    "Agent conversation: {} · {used}/{limit} turns",
                    if paused {
                        "paused"
                    } else if enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(240.)).child(Input::new(&self.turn_limit)))
                        .child(
                            Button::new("channel-auto-enable")
                                .label("Enable or resume")
                                .disabled(self.busy)
                                .on_click(cx.listener(|pane, _, _, cx| {
                                    pane.configure_conversation(true, cx)
                                })),
                        )
                        .child(
                            Button::new("channel-auto-disable")
                                .label("Disable")
                                .disabled(self.busy)
                                .on_click(cx.listener(|pane, _, _, cx| {
                                    pane.configure_conversation(false, cx)
                                })),
                        )
                        .child(
                            Button::new("channel-auto-stop")
                                .label("Stop conversation")
                                .disabled(self.busy)
                                .on_click(cx.listener(move |pane, _, _, cx| {
                                    pane.command(
                                        "stop_channel_agent_conversation",
                                        json!({"channelId":channel_id}),
                                        ResultKind::Snapshot,
                                        cx,
                                    )
                                })),
                        ),
                );

        let has_earlier = match self.next_before.get(&field(&channel, "id")) {
            Some(next) => next.is_some(),
            None => true,
        };
        let id = field(&channel, "id");
        let state = self.transcripts.entry(id.clone()).or_default();
        let messages = state.messages.clone();
        let list_state = state.list.clone();
        let palette = crate::design::palette(&self.snapshot["settings"], window.appearance());
        let agents = rows(&self.snapshot, "agents");
        let mut transcript = div().flex_1().min_h(px(0.)).flex().flex_col().gap_2();
        if has_earlier {
            transcript = transcript.child(
                Button::new("channel-history")
                    .label("Load earlier messages")
                    .disabled(self.busy)
                    .on_click(cx.listener(|pane, _, _, cx| pane.load_earlier(cx))),
            );
        }
        let held = state.has_pending_live();
        transcript = transcript.child(
            Button::new("channel-jump-live")
                .label(if held {
                    "Show new messages"
                } else {
                    "Jump to latest"
                })
                .on_click(cx.listener(move |pane, _, _, cx| {
                    if let Some(state) = pane.transcripts.get_mut(&id) {
                        state.release_live();
                        state.list.scroll_to_end();
                    }
                    cx.notify();
                })),
        );
        if messages.is_empty() {
            transcript = transcript.child("No messages in this channel yet.");
        }
        transcript = transcript.child(
            list(list_state, move |index, _, _| {
                let message = &messages[index];
                let author = message["authorName"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        if field(message, "role") == "user" {
                            "You".into()
                        } else {
                            agents
                                .iter()
                                .find(|a| field(a, "id") == field(message, "agentId"))
                                .map(|a| field(a, "name"))
                                .unwrap_or_else(|| "Agent".into())
                        }
                    });
                let mut item = div()
                    .p_3()
                    .mb_2()
                    .rounded_md()
                    .bg(crate::design::surface_color(palette.card))
                    .text_color(rgb(palette.ink))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().text_color(rgb(palette.muted)).child(author))
                    .child(
                        TextView::markdown(
                            SharedString::from(format!("channel-message-{}", field(message, "id"))),
                            field(message, "text"),
                        )
                        .selectable(true)
                        .scrollable(false),
                    );
                for attachment in rows(message, "attachments") {
                    item = item.child(format!(
                        "Attachment: {} (preview pending)",
                        field(&attachment, "name")
                    ));
                }
                item.into_any_element()
            })
            .size_full(),
        );
        panel
            .child(transcript)
            .child(Textarea::new(&self.composer).h(px(90.)).disabled(self.busy))
            .child(
                Button::new("channel-send")
                    .label("Send to selected recipients")
                    .disabled(self.busy || self.recipients.is_empty())
                    .on_click(cx.listener(|pane, _, _, cx| pane.send(cx))),
            )
            .into_any_element()
    }
}
impl Render for ChannelPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.selection_removed) {
            self.select(None, window, cx);
        }

        if let Some((channel, draft)) = self.clear_sent.take() {
            if self.selected.as_ref() == Some(&channel)
                && self.composer.read(cx).value().to_string() == draft
            {
                self.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.drafts.remove(&channel);
            } else if self.drafts.get(&channel) == Some(&draft) {
                self.drafts.remove(&channel);
            }
        }
        let mut sidebar = div()
            .id("native-channel-list")
            .w(px(220.))
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                Button::new("channel-new")
                    .label("New channel")
                    .disabled(self.busy)
                    .on_click(cx.listener(|pane, _, window, cx| pane.create(window, cx))),
            );
        for channel in self.channels() {
            let id = field(&channel, "id");
            let selected = self.selected.as_ref() == Some(&id);
            sidebar = sidebar.child(
                Button::new(SharedString::from(format!("channel-select-{id}")))
                    .label(format!(
                        "{}{}",
                        if selected { "• " } else { "" },
                        field(&channel, "name")
                    ))
                    .on_click(cx.listener(move |pane, _, window, cx| {
                        pane.select(Some(id.clone()), window, cx)
                    })),
            );
        }
        let body = if self.creating || self.selected.is_some() {
            self.render_channel(window, cx)
        } else {
            div()
                .child("Select a channel or create one.")
                .into_any_element()
        };
        let mut content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .gap_2();
        if let Some(error) = &self.error {
            content = content.child(div().text_color(rgb(0xf87171)).child(error.clone()));
        }
        if let Some(notice) = &self.notice {
            content = content.child(notice.clone());
        }
        div()
            .flex()
            .size_full()
            .gap_3()
            .p_4()
            .child(sidebar)
            .child(content.child(body))
    }
}

#[cfg(test)]
mod native_channel_tests {
    use super::*;
    use ::core::prelude::v1::test;
    #[::core::prelude::v1::test]
    fn channel_sync_scope_ignores_task_tokens_but_keeps_members_history_and_resets() {
        let previous = json!({"channels":[{"id":"a","agentIds":["one"],"messages":[{"id":"m","text":"hello"}]}],"agents":[{"id":"one","name":"One","internal":false,"status":"idle"}],"tasks":[{"id":"t","messages":[]}]});
        let mut next = previous.clone();
        next["tasks"][0]["messages"] = json!([{"text":"new token"}]);
        next["agents"][0]["status"] = json!("running");
        assert!(!channel_metadata_changed(&previous, &next));
        assert!(!channel_agents_changed(&previous, &next));
        next["channels"][0]["agentIds"] = json!([]);
        assert!(channel_metadata_changed(&previous, &next));
        assert!(!channel_messages_changed(Some(&previous["channels"][0]), &next["channels"][0], false));
        assert!(channel_messages_changed(Some(&previous["channels"][0]), &next["channels"][0], true));
        next["channels"][0]["messages"][0]["text"] = json!("edited");
        let mut body_only = previous.clone();
        body_only["channels"][0]["messages"][0]["text"] = json!("edited");
        assert!(!channel_metadata_changed(&previous, &body_only), "metadata gate must never traverse transcript bodies");
        assert!(channel_messages_changed(Some(&previous["channels"][0]), &next["channels"][0], false));
        next["agents"][0]["name"] = json!("Renamed");
        assert!(channel_agents_changed(&previous, &next));
    }

    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn channel_sync_keeps_paged_state_on_unrelated_updates_and_honors_explicit_reset(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| ChannelPane::new(DesktopService::disconnected_for_ui_fixture(), window, cx));
            retained = Some(pane.clone());
            gpui_kit::component::Root::new(pane, window, cx)
        });
        let pane = retained.unwrap();
        let previous = Rc::new(json!({"channels":[{"id":"a","messages":[{"id":"m","text":"hello"}]}]}));
        let cursors = HashMap::from([("a".into(), Some("m".into()))]);
        visual.update(|_, cx| pane.update(cx, |pane, cx| pane.sync(previous.clone(), "one", false, &[], &cursors, cx)));
        let mut next = previous.as_ref().clone();
        next["tasks"] = json!([{"id":"t","messages":[{"text":"token"}]}]);
        let next = Rc::new(next);
        visual.update(|_, cx| pane.update(cx, |pane, cx| {
            let state = pane.transcripts.get_mut("a").unwrap();
            state.boundary = crate::transcript::HistoryBoundary::More("m".into());
            let messages = state.messages.clone();
            pane.sync_scoped(next.clone(), "two", false, &[], &cursors, &HashSet::new(), cx);
            assert!(Rc::ptr_eq(&pane.snapshot, &next));
            assert_eq!(pane.revision, "two");
            assert!(Rc::ptr_eq(&messages, &pane.transcripts["a"].messages));
            assert!(matches!(&pane.transcripts["a"].boundary, crate::transcript::HistoryBoundary::More(cursor) if cursor == "m"));
            // Even a different body must be ignored when the delta says no
            // channel changed: this verifies the scoped path avoids deep scans.
            let mut body = next.as_ref().clone();
            body["channels"][0]["messages"][0]["text"] = json!("edited");
            let body = Rc::new(body);
            pane.sync_scoped(body.clone(), "three", false, &[], &cursors, &HashSet::new(), cx);
            assert!(Rc::ptr_eq(&messages, &pane.transcripts["a"].messages));
            // An explicit reset is authoritative even with no changed IDs.
            pane.sync_scoped(body, "four", false, &["a".into()], &HashMap::new(), &HashSet::new(), cx);
            assert!(matches!(pane.transcripts["a"].boundary, crate::transcript::HistoryBoundary::Unknown));
            assert!(pane.next_before.is_empty());
        }));
    }

    #[::core::prelude::v1::test]
    fn reset_and_cursor_removal_invalidate_only_affected_scopes() {
        let snapshot = json!({"channels":[{"id":"a","messages":[{"id":"known"}]},{"id":"b","messages":[]},{"id":"c","messages":[]}]});
        let mut cursors = HashMap::from([
            ("a".into(), Some("known".into())),
            ("b".into(), None),
            ("c".into(), Some("deleted".into())),
            ("gone".into(), None),
        ]);
        reconcile_cursors(&mut cursors, &snapshot, false, &["a".into()]);
        assert_eq!(cursors, HashMap::from([("b".into(), None)]));
        assert!(
            !cursors.contains_key("c"),
            "deleted cursor means unknown, not complete"
        );
        reconcile_cursors(&mut cursors, &snapshot, true, &[]);
        assert!(cursors.is_empty());
    }
}
