//! Retained notification inbox. Actions are intents; backend ownership stays with Desktop.
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme as _, Disableable, Icon, Selectable, Sizable, ThemeStyled as _, button::*,
        popover::Popover,
    },
    prelude::*,
    *,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Debug, PartialEq)]
pub enum NotificationKind {
    Approval { request: Value },
    Message,
    Attention,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationTone {
    Info,
    Success,
    Warning,
    Danger,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationPersistence {
    Fixed,
    Transient,
}
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationEntry {
    pub id: String,
    pub created_at: i64,
    pub task_id: String,
    pub channel_id: Option<String>,
    pub origin: String,
    pub tone: NotificationTone,
    pub title: String,
    pub detail: String,
    pub message_id: Option<String>,
    pub summary: Option<String>,
    pub kind: NotificationKind,
}
impl NotificationEntry {
    pub fn persistence(&self) -> NotificationPersistence {
        match self.kind {
            NotificationKind::Approval { .. } => NotificationPersistence::Fixed,
            _ => NotificationPersistence::Transient,
        }
    }
    fn formatted_detail(&self) -> String {
        let NotificationKind::Approval { request } = &self.kind else {
            return self.detail.clone();
        };
        let parsed = request
            .get("detail")
            .and_then(Value::as_str)
            .and_then(|detail| serde_json::from_str::<Value>(detail).ok());
        let Some(payload) = parsed else {
            return self.detail.clone();
        };
        let command = match &payload["command"] {
            Value::String(command) => command.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            _ => return self.detail.clone(),
        };
        if command.is_empty() {
            return self.detail.clone();
        }
        if self.detail.trim() == command.trim() {
            return format!("$ {command}");
        }
        format!("{}\n$ {}", self.detail, command)
    }
    fn icon(&self) -> IconName {
        match &self.kind {
            NotificationKind::Approval { request }
                if request.get("input").is_some_and(|input| !input.is_null()) =>
            {
                IconName::MessageSquare
            }
            NotificationKind::Approval { .. } => IconName::ShieldCheck,
            NotificationKind::Message => IconName::MessageSquare,
            NotificationKind::Attention if self.tone == NotificationTone::Success => {
                IconName::Check
            }
            NotificationKind::Attention => IconName::TriangleAlert,
        }
    }
}

fn intro(detail: &str) -> String {
    let compact = detail.trim().to_owned();
    let mut chars = compact.chars();
    let mut excerpt: String = chars.by_ref().take(60).collect();
    if chars.next().is_some() {
        if let Some(boundary) = excerpt.rfind(char::is_whitespace) {
            excerpt.truncate(boundary);
        }
        excerpt.push('…');
    }
    excerpt
}

// Use the same 12/14 text role as the model and permission popups.
fn notification_font(cx: &App) -> Pixels {
    cx.theme().font_size * (12. / 14.)
}

fn can_expand(detail: &str) -> bool {
    detail.trim().chars().count() > 60
}

fn detail_parts(detail: &str) -> Vec<(bool, String)> {
    let mut parts = Vec::new();
    let mut fenced = false;
    for line in detail.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let command = line
            .strip_prefix("$ ")
            .or_else(|| line.strip_prefix("Command: "));
        let first = line.split_whitespace().next().unwrap_or("");
        let shell = matches!(
            first,
            "git"
                | "npm"
                | "npx"
                | "pnpm"
                | "yarn"
                | "cargo"
                | "rustup"
                | "node"
                | "python"
                | "python3"
                | "bash"
                | "sh"
                | "zsh"
                | "cd"
                | "ls"
                | "rg"
                | "curl"
                | "docker"
                | "make"
                | "sudo"
        ) && line.contains(' ');
        if fenced || command.is_some() || shell {
            parts.push((true, command.unwrap_or(line).to_owned()));
        } else {
            for (index, part) in line.split('`').enumerate() {
                if !part.trim().is_empty() {
                    parts.push((index % 2 == 1, part.to_owned()));
                }
            }
        }
    }
    parts
}

fn notification_detail(detail: &str, summary: Option<&str>, expanded: bool, cx: &App) -> Div {
    let visible = if expanded {
        detail.to_owned()
    } else {
        summary.map(str::to_owned).unwrap_or_else(|| intro(detail))
    };
    let font = notification_font(cx);
    let mut body = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(3.))
        .text_size(font * 0.9)
        .text_color(cx.theme().muted_foreground);
    for (code, text) in detail_parts(&visible) {
        body = body.child(
            div()
                .min_w_0()
                .when(code, |part| {
                    part.debug_selector(|| "notification-code".into())
                        .font_family(crate::design::FONT_MONO)
                        .text_size(font * 0.8)
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .bg(rgba(0x00000024))
                })
                .child(text),
        );
    }
    body
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationEvent {
    Review {
        task_id: String,
        request_id: Option<String>,
    },
    ReviewChannel {
        channel_id: String,
    },
    Dismiss {
        id: String,
    },
    Decide {
        approval_id: String,
        task_id: String,
        decision: String,
    },
}
#[derive(Default)]
struct RequestState {
    busy: bool,
    error: Option<String>,
}
pub struct NotificationBell {
    entries: Vec<NotificationEntry>,
    // Approval progress belongs to the request, shared by every filtered view.
    requests: Rc<RefCell<HashMap<String, RequestState>>>,
    workspace: Option<(String, String)>,
    open: bool,
    expanded: HashSet<String>,
}
impl EventEmitter<NotificationEvent> for NotificationBell {}
impl NotificationBell {
    pub fn new(_: &mut Context<Self>) -> Self {
        Self {
            entries: Vec::new(),
            requests: Default::default(),
            workspace: None,
            open: false,
            expanded: HashSet::new(),
        }
    }
    pub(super) fn for_workspace(id: String, name: String, global: &Self) -> Self {
        Self {
            entries: Vec::new(), requests: global.requests.clone(),
            workspace: Some((id, name)), open: false, expanded: HashSet::new(),
        }
    }
    pub(super) fn set_workspace_name(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some((_, current)) = &mut self.workspace {
            if current != name { *current = name.into(); cx.notify(); }
        }
    }
    pub fn set_entries(&mut self, entries: Vec<NotificationEntry>, cx: &mut Context<Self>) {
        if entries.is_empty() && self.open {
            self.open = false;
            cx.notify();
        }
        if self.entries == entries {
            return;
        }
        if self.workspace.is_none() {
            self.requests.borrow_mut().retain(|id, _| entries.iter().any(|entry| &entry.id == id));
        }
        self.expanded
            .retain(|id| entries.iter().any(|entry| &entry.id == id));
        self.entries = entries;
        cx.notify();
    }
    /// Completion/error belongs to exactly one request. Snapshot removal clears its state.
    pub fn set_request_state(
        &mut self,
        id: &str,
        busy: bool,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self.entries.iter().find(|entry| entry.id == id || matches!(&entry.kind, NotificationKind::Approval { request } if request["id"].as_str() == Some(id))) {
            self.requests.borrow_mut().insert(entry.id.clone(), RequestState { busy, error });
            cx.notify();
        }
    }
    pub fn has_notifications(&self) -> bool {
        !self.entries.is_empty()
    }
    fn decide(&mut self, id: &str, decision: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
            return;
        };
        let NotificationKind::Approval { request } = &entry.kind else {
            return;
        };
        if self.requests.borrow().get(id).is_some_and(|state| state.busy)
            || !crate::approval_prompt::allowed_decisions(request)
                .iter()
                .any(|(value, _)| *value == decision)
            || !matches!(decision, "approve_once" | "deny")
        {
            return;
        }
        let event = NotificationEvent::Decide {
            approval_id: request["id"].as_str().unwrap_or(id).into(),
            task_id: entry.task_id.clone(),
            decision: decision.into(),
        };
        self.requests.borrow_mut().insert(
            id.into(),
            RequestState {
                busy: true,
                error: None,
            },
        );
        cx.emit(event);
        cx.notify();
    }
    fn review(&mut self, id: &str, open_request: bool, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
            return;
        };
        if let Some(channel_id) = &entry.channel_id {
            cx.emit(NotificationEvent::ReviewChannel {
                channel_id: channel_id.clone(),
            });
        } else {
            cx.emit(NotificationEvent::Review {
                task_id: entry.task_id.clone(),
                request_id: (open_request
                    && matches!(entry.kind, NotificationKind::Approval { .. }))
                .then(|| match &entry.kind {
                    NotificationKind::Approval { request } => {
                        request["id"].as_str().unwrap_or(&entry.id).to_owned()
                    }
                    _ => entry.id.clone(),
                }),
            });
        }
        self.open = false;
        cx.notify();
    }
    fn toggle_expanded(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.entries.iter().any(|entry| {
            entry.id == id
                && (entry
                    .summary
                    .as_deref()
                    .is_some_and(|summary| summary.trim() != entry.formatted_detail().trim())
                    || can_expand(&entry.formatted_detail()))
        }) {
            return;
        }
        if !self.expanded.remove(id) {
            self.expanded.insert(id.into());
        }
        cx.notify();
    }
    fn dismiss(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.entries.iter().any(|entry| {
            entry.id == id && entry.persistence() == NotificationPersistence::Transient
        }) {
            cx.emit(NotificationEvent::Dismiss { id: id.into() });
            if self.entries.len() == 1 {
                self.open = false;
                cx.notify();
            }
        }
    }
    fn actions(&self, entry: &NotificationEntry, expanded: bool, cx: &mut Context<Self>) -> Div {
        let id = &entry.id;
        let font = notification_font(cx);
        let busy = self.requests.borrow().get(id).is_some_and(|state| state.busy);
        let mut actions = div()
            .flex()
            .gap(px(4.))
            .when(!expanded, |this| this.flex_col())
            .when(expanded, |this| this.flex_wrap());
        if let NotificationKind::Approval { request } = &entry.kind {
            let decisions: Vec<_> = crate::approval_prompt::allowed_decisions(request)
                .into_iter()
                .filter(|(decision, _)| matches!(*decision, "approve_once" | "deny"))
                .collect();
            if decisions.is_empty() {
                let action_id = id.clone();
                let selector = format!("notification-reply-{id}");
                actions = actions.child(
                    div().debug_selector(move || selector.clone()).child(
                        Button::new(SharedString::from(format!("notification-reply-{id}")))
                            .icon(IconName::Reply)
                            .ghost()
                            .compact()
                            .text_size(font)
                            .when(expanded, |button| {
                                button.child(div().text_size(font).child("Reply")).outline()
                            })
                            .when(!expanded, |button| button.size(px(26.)))
                            .tooltip("Reply in pane")
                            .accessibility_label("Reply in pane")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                cx.stop_propagation();
                                view.review(&action_id, true, cx);
                            })),
                    ),
                );
            }
            for (decision, label) in decisions {
                let action_id = id.clone();
                let selector = format!("notification-{decision}-{id}");
                actions = actions.child(
                    div().debug_selector(move || selector.clone()).child(
                        Button::new(SharedString::from(format!("notification-{decision}-{id}")))
                            .icon(if decision == "approve_once" {
                                IconName::Check
                            } else {
                                IconName::X
                            })
                            .ghost()
                            .compact()
                            .text_size(font)
                            .disabled(busy)
                            .when(expanded, |button| {
                                button.child(div().text_size(font).child(label)).outline()
                            })
                            .when(!expanded, |button| button.size(px(26.)))
                            .tooltip(label)
                            .accessibility_label(label)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                cx.stop_propagation();
                                view.decide(&action_id, decision, cx);
                            })),
                    ),
                );
            }
        } else {
            // Transient updates have no approval/review actions. The row itself navigates.
            let dismiss_id = id.clone();
            let selector = format!("notification-dismiss-{id}");
            actions = actions.child(
                div().debug_selector(move || selector.clone()).child(
                    Button::new(SharedString::from(format!("notification-dismiss-{id}")))
                        .icon(IconName::X)
                        .ghost()
                        .compact()
                        .size(px(26.))
                        .tooltip("Dismiss")
                        .accessibility_label("Dismiss notification")
                        .on_click(cx.listener(move |view, _, _, cx| {
                            cx.stop_propagation();
                            view.dismiss(&dismiss_id, cx);
                        })),
                ),
            );
        }
        actions
    }
    fn content(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let font = notification_font(cx);
        let mut list = div()
            .id("notification-list")
            .debug_selector(|| "notification-list".into())
            .w_full()
            .max_h(px(480.))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for entry in &self.entries {
            let id = entry.id.clone();
            let detail = entry.formatted_detail();
            let expandable = entry
                .summary
                .as_deref()
                .is_some_and(|summary| summary.trim() != detail.trim())
                || can_expand(&detail);
            let expanded = expandable && self.expanded.contains(&id);
            let fixed = entry.persistence() == NotificationPersistence::Fixed;
            let tint = match entry.tone {
                NotificationTone::Info => cx.theme().primary,
                NotificationTone::Success => cx.theme().success,
                NotificationTone::Warning => cx.theme().warning,
                NotificationTone::Danger => cx.theme().danger,
            };
            let mut background = tint;
            background.a = 0.055;
            let mut hover = tint;
            hover.a = 0.10;
            let mut text = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(font * 0.8)
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .truncate()
                                .children(
                                    entry
                                        .origin
                                        .split_once(" | ")
                                        .map(|(agent, chat)| {
                                            vec![
                                                div().flex_shrink_0().child(agent.to_owned()),
                                                div().min_w_0().truncate().child(chat.to_owned()),
                                            ]
                                        })
                                        .unwrap_or_else(|| {
                                            vec![
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(entry.origin.clone()),
                                            ]
                                        }),
                                ),
                        )
                        .child(
                            div().flex_shrink_0().child(
                                crate::timestamps::local_datetime(entry.created_at)
                                    .map(|time| time.format("%-I:%M %P").to_string())
                                    .unwrap_or_default(),
                            ),
                        ),
                )
                .when(!matches!(entry.kind, NotificationKind::Message), |row| {
                    row.child(
                        div()
                            .w_full()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(font)
                            .truncate()
                            .child(entry.title.clone()),
                    )
                })
                .child(notification_detail(
                    &detail,
                    entry.summary.as_deref(),
                    expanded,
                    cx,
                ));
            if self.requests.borrow().get(&id).is_some_and(|state| state.busy) {
                text = text.child(
                    div()
                        .text_size(font * 0.9)
                        .text_color(cx.theme().muted_foreground)
                        .child("Waiting for response…"),
                );
            }
            if let Some(error) = self
                .requests
                .borrow()
                .get(&id)
                .and_then(|state| state.error.clone())
            {
                text = text.child(
                    div()
                        .text_size(font * 0.9)
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                );
            }
            if expanded && fixed {
                text = text.child(div().pt(px(6.)).child(self.actions(entry, true, cx)));
            }
            let toggle_id = id.clone();
            let toggle_selector = format!("notification-expand-{id}");
            let mut rail = div().flex_shrink_0().flex().flex_col().gap(px(3.));
            if !expanded || !fixed {
                rail = rail.child(self.actions(entry, false, cx));
            }
            let toggle = div().debug_selector(move || toggle_selector.clone()).child(
                Button::new(SharedString::from(format!("notification-expand-{id}")))
                    .icon(if expanded {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .ghost()
                    .compact()
                    .size(px(26.))
                    .tooltip(if expanded {
                        "Collapse"
                    } else {
                        "Expand notification"
                    })
                    .accessibility_label(if expanded {
                        "Collapse notification"
                    } else {
                        "Expand notification"
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.toggle_expanded(&toggle_id, cx);
                    })),
            );
            let review_id = id.clone();
            let row_selector = format!("notification-row-{id}");
            list = list.child(
                div()
                    .id(SharedString::from(row_selector.clone()))
                    .debug_selector(move || row_selector.clone())
                    .w_full()
                    .flex_shrink_0()
                    .flex()
                    .items_stretch()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(background)
                    .hover(move |style| style.bg(hover))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| view.review(&review_id, false, cx)))
                    .child(
                        div()
                            .w(px(26.))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .pt(px(2.))
                            .child(Icon::new(entry.icon()).with_size(px(17.)).text_color(tint))
                            .when(expandable, |column| column.child(toggle)),
                    )
                    .child(text)
                    .child(rail),
            );
        }
        // Own the surface so the vendor popover's default body padding cannot
        // inset the full-width rows. Native overlay/focus handling is retained.
        div()
            .debug_selector(|| "notification-surface".into())
            .w(px(400.).min((window.viewport_size().width - px(16.)).max(px(1.))))
            .popover_style(cx)
            .overflow_hidden()
            .font_family(cx.theme().font_family.clone())
            .text_size(font)
            .when_some(self.workspace.as_ref(), |surface, (_, name)| surface.child(
                div().px(px(12.)).py(px(8.)).border_b_1().border_color(cx.theme().border)
                    .text_size(font).text_color(cx.theme().muted_foreground).truncate()
                    .child(format!("{name} · Notifications")),
            ))
            .child(list)
            .into_any_element()
    }
}
// A retained decorative child owns pulse frames; no task/model polling or
// notifications of the workspace host are needed to animate the badge.
struct WorkspaceNotificationGlow { tint: Hsla, pulsing: bool }
impl Render for WorkspaceNotificationGlow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut soft = self.tint;
        soft.a = 0.28;
        let glow = div().absolute().inset_0().rounded_full()
            .bg(soft).shadow(vec![BoxShadow::new(px(0.), px(0.), soft)
                .blur_radius(crate::design::ui_scaled_px(4.)).spread_radius(crate::design::ui_scaled_px(1.))]);
        if self.pulsing && !cx.reduce_motion() && crate::native_motion::motion_allowed() {
            glow.with_animation("workspace-notification-pulse",
                Animation::new(std::time::Duration::from_millis(2200)).repeat_synced().with_max_fps(12.),
                |glow, phase| glow.opacity(0.35 + 0.45 * (1. - (phase * std::f32::consts::TAU).cos()) / 2.))
                .into_any_element()
        } else { glow.opacity(0.55).into_any_element() }
    }
}
fn workspace_notification_glow(id: &str, tint: Hsla, pulsing: bool, window: &mut Window, cx: &mut App) -> Entity<WorkspaceNotificationGlow> {
    let holder = window.use_keyed_state(SharedString::from(format!("workspace-notification-glow-{id}")), cx,
        |_, cx| cx.new(|_| WorkspaceNotificationGlow { tint, pulsing }));
    let glow = holder.read(cx).clone();
    glow.update(cx, |glow, cx| {
        if glow.tint != tint || glow.pulsing != pulsing {
            glow.tint = tint; glow.pulsing = pulsing; cx.notify();
        }
    });
    glow
}

#[derive(IntoElement)]
struct NotificationTrigger {
    count: usize,
    open: bool,
    workspace: Option<(String, String)>,
    action_needed: bool,
}
impl Selectable for NotificationTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }
    fn is_selected(&self) -> bool {
        self.open
    }
}
impl RenderOnce for NotificationTrigger {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let count = self.count;
        if let Some((id, name)) = self.workspace {
            let tint = if self.action_needed { cx.theme().warning } else { cx.theme().primary };
            let mut background = tint;
            background.a = if self.open { 0.18 } else { 0.09 };
            let mut hover = tint;
            hover.a = 0.16;
            let selector = format!("workspace-notifications-{id}");
            let diameter = crate::design::ui_scaled_px(16.);
            let glow = workspace_notification_glow(&id, tint, !self.open, window, cx);
            return div().id(SharedString::from(selector.clone())).relative().flex_shrink_0().size(diameter)
                .debug_selector(move || selector.clone())
                .child(glow)
                .child(Button::new(SharedString::from(format!("workspace-notifications-button-{id}")))
                    .custom(ButtonCustomVariant::new(cx).color(background).foreground(tint).hover(hover).active(hover))
                    .compact().selected(self.open)
                    .w(diameter).min_w(diameter).max_w(diameter).h(diameter).p_0().rounded_full()
                    .text_size(crate::design::ui_scaled_px(9.))
                    .label(if count > 9 { "9+".into() } else { count.to_string() })
                    .accessibility_label(format!("{count} {} in {name} workspace", if count == 1 { "notification" } else { "notifications" }))
                    .tooltip(format!("{name} · {count} notifications"))
                    .on_click(|_, _, cx| cx.stop_propagation()))
                .into_any_element();
        }
        let mut trigger = div()
            .id("notification-trigger")
            .relative()
            .debug_selector(|| "notification-trigger".into())
            .child(
                Button::new("notification-bell")
                    .accessibility_label("Notifications")
                    .accessibility_id("notification-bell")
                    .ghost()
                    .selected(self.open)
                    .tooltip(if count > 0 {
                        "Pending notifications"
                    } else {
                        "Notifications"
                    })
                    .child(
                        Icon::new(IconName::Bell)
                            .with_size(px(18.))
                            .text_color(cx.theme().muted_foreground),
                    ),
            );
        if count > 0 {
            // Outside the button's clipping box, above its top-right corner.
            trigger = trigger.child(
                div()
                    .debug_selector(|| "notification-count".into())
                    .absolute()
                    .top(px(-4.))
                    .right(px(-5.))
                    .min_w(px(16.))
                    .h(px(16.))
                    .px(px(4.))
                    .rounded_full()
                    .bg(cx.theme().primary)
                    .text_color(cx.theme().primary_foreground)
                    .font_family(cx.theme().font_family.clone())
                    .text_size(cx.theme().font_size * 0.8)
                    .font_weight(FontWeight::SEMIBOLD)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if count > 99 {
                        "99+".into()
                    } else {
                        count.to_string()
                    }),
            );
        }
        trigger.into_any_element()
    }
}
impl Render for NotificationBell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity();
        let content_owner = owner.clone();
        let count = self.entries.len();
        let trigger = NotificationTrigger {
            count, open: self.open, workspace: self.workspace.clone(),
            action_needed: self.entries.iter().any(|entry| matches!(entry.kind, NotificationKind::Approval { .. })),
        };
        if count == 0 {
            return if self.workspace.is_some() { div().into_any_element() } else { trigger.into_any_element() };
        }
        let popover_id = self.workspace.as_ref().map(|(id, _)| format!("workspace-notification-popover-{id}"))
            .unwrap_or_else(|| "notification-popover".into());
        Popover::new(SharedString::from(popover_id))
            .appearance(false)
            .anchor(Anchor::TopRight)
            .open(self.open)
            .overlay_closable(true)
            .trigger(trigger)
            .on_open_change(move |open, _, cx| {
                owner.update(cx, |view, cx| {
                    view.open = *open && view.has_notifications();
                    cx.notify();
                })
            })
            .content(move |_, window, cx| content_owner.update(cx, |view, cx| view.content(window, cx)))
            .into_any_element()
    }
}

/// An explicit development fixture: real native rendering, no backend or profile.
#[cfg(feature = "visual-export")]
pub fn open_preview_if_requested(cx: &mut App) -> bool {
    let Ok(mode) = std::env::var("MONITTER_GPUI_NOTIFICATION_PREVIEW") else {
        return false;
    };
    let _ = monitter_gpui_desktop::fonts::register(cx);
    gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
    let expanded = mode == "expanded";
    struct Preview {
        bell: Entity<NotificationBell>,
    }
    impl Render for Preview {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .bg(cx.theme().background)
                .text_color(cx.theme().foreground)
                .font_family(cx.theme().font_family.clone())
                .text_size(cx.theme().font_size)
                .p(px(24.))
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child("Notifications · visual fixture")
                        .child(self.bell.clone()),
                )
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child("Synthetic examples · no live sessions"),
                )
                .child(self.bell.update(cx, |bell, cx| bell.content(window, cx)))
        }
    }
    let (window, _) = gpui_kit::open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(500.),px(650.)),cx))),
        titlebar: Some(TitlebarOptions {title:Some("Monitter Notification Preview".into()), ..Default::default()}),
        ..Default::default()
    }, cx, |_,cx| {
        let bell = cx.new(|cx| {
            let mut bell = NotificationBell::new(cx);
            bell.open = false;
            bell.entries = vec![
                NotificationEntry { id:"preview-approval".into(), created_at:chrono::Utc::now().timestamp_millis(), task_id:"preview-one".into(), channel_id:None,
                    origin:"Codex | New layout for panes and notifications".into(), tone:NotificationTone::Warning,
                    title:"Approval needed".into(), detail:"Run the focused native interface tests before preparing the desktop build. This checks pane navigation, notification actions, and retained state without touching your running sessions.".into(),
                    message_id:None,summary:None,kind:NotificationKind::Approval {request:serde_json::json!({"id":"preview-approval","status":"pending","input":null,"detail":"{\"command\":\"cargo test --bin monitter-gpui-desktop notification\"}"})}},
                NotificationEntry {id:"preview-reply".into(),created_at:chrono::Utc::now().timestamp_millis(),task_id:"preview-two".into(),channel_id:None,
                    origin:"Codex | Refine the notification dropdown".into(),tone:NotificationTone::Info,title:"".into(),
                    detail:"The new rows now extend across the dropdown. Each notification has an icon, an origin label, and a short introduction. Expand this update to read the complete message, or click the row to return to its pane.".into(),message_id:None,summary:None,kind:NotificationKind::Message},
                NotificationEntry {id:"preview-done".into(),created_at:chrono::Utc::now().timestamp_millis(),task_id:"preview-three".into(),channel_id:None,
                    origin:"Codex | Verify pane movement".into(),tone:NotificationTone::Success,title:"Completed".into(),
                    detail:"All focused pane movement checks passed.".into(),message_id:None,summary:None,kind:NotificationKind::Attention},
            ];
            if expanded { bell.expanded.insert("preview-approval".into()); }
            bell
        });
        cx.subscribe(&bell, |bell,event:&NotificationEvent,cx| {
            if let NotificationEvent::Dismiss {id} = event {
                bell.update(cx,|bell,cx| {let entries=bell.entries.iter().filter(|entry| &entry.id!=id).cloned().collect();bell.set_entries(entries,cx);});
            }
        }).detach();
        cx.new(|_|Preview {bell})
    }).expect("notification preview window");
    crate::visual_export::install(cx);
    crate::visual_export::schedule(window, cx);
    cx.activate(true);
    true
}

#[cfg(all(test, feature = "visual-export"))]
mod tests {
    use super::{
        NotificationBell, NotificationEntry, NotificationEvent, NotificationKind, NotificationTone,
    };
    use gpui_kit::{AppContext as _, Modifiers, TestAppContext};
    use std::{cell::RefCell, rc::Rc};
    fn approval(id: &str, input: serde_json::Value) -> NotificationEntry {
        NotificationEntry {
            id: id.into(),
            created_at: chrono::Utc::now().timestamp_millis(),
            task_id: "task".into(),
            channel_id: None,
            origin: "Codex | Test task".into(),
            tone: NotificationTone::Warning,
            title: "Tool approval".into(),
            detail: "Run the requested command".into(),
            message_id: None,
            summary: None,
            kind: NotificationKind::Approval {
                request: serde_json::json!({"id":id,"status":"pending","input":input}),
            },
        }
    }
    #[test]
    fn short_messages_have_no_expansion_and_commands_are_recognized() {
        assert!(!super::can_expand("All checks passed."));
        let mut entry = approval("command", serde_json::Value::Null);
        entry.kind = NotificationKind::Approval {
            request: serde_json::json!({"detail":"{\"command\":[\"git\",\"status\"]}"}),
        };
        assert!(entry.formatted_detail().ends_with("\n$ git status"));
        assert!(super::can_expand(&"long summary ".repeat(20)));
        assert_eq!(
            super::detail_parts("Run this:\n```sh\ngit status --short\n```"),
            vec![
                (false, "Run this:".into()),
                (true, "git status --short".into())
            ]
        );
        assert_eq!(
            super::detail_parts("npm run check"),
            vec![(true, "npm run check".into())]
        );
        assert_eq!(
            super::detail_parts("The command finished."),
            vec![(false, "The command finished.".into())]
        );
    }
    #[cfg(feature = "performance-profiling")]
    #[gpui_kit::test]
    fn workspace_notification_pulse_ticks_locally_and_reduced_motion_stays_quiet(cx: &mut TestAppContext) {
        cx.update(|cx| { gpui_kit::init(cx); crate::native_motion::initialize_for_ui_fixture(cx); });
        let (glow, visual) = cx.add_window_view(|_, _| super::WorkspaceNotificationGlow {
            tint: gpui_kit::rgb(0x6594db).into(), pulsing: true,
        });
        let notifications = Rc::new(std::cell::Cell::new(0));
        visual.update(|window, cx| {
            let count = notifications.clone();
            cx.observe(&glow, move |_, _| count.set(count.get() + 1)).detach();
            window.draw(cx).clear(cx);
        });
        visual.run_until_parked();
        let initial = notifications.get();
        visual.executor().advance_clock(std::time::Duration::from_millis(100));
        visual.run_until_parked();
        assert!(notifications.get() > initial, "visible pulse schedules a local frame");
        visual.update(|window, cx| { cx.set_reduce_motion(true); window.draw(cx).clear(cx); });
        visual.executor().advance_clock(std::time::Duration::from_millis(100));
        visual.run_until_parked();
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let reduced = notifications.get();
        visual.executor().advance_clock(std::time::Duration::from_secs(1));
        visual.run_until_parked();
        assert_eq!(notifications.get(), reduced, "steady reduced-motion glow requests no more frames");
    }

    #[gpui_kit::test]
    fn workspace_notification_views_share_request_progress_and_errors(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            let global = cx.new(NotificationBell::new);
            global.update(cx, |view, cx| view.set_entries(vec![approval("one", serde_json::Value::Null)], cx));
            let scoped = cx.new(|cx| NotificationBell::for_workspace("workspace".into(), "Workspace".into(), global.read(cx)));
            scoped.update(cx, |view, cx| {
                view.set_entries(vec![approval("one", serde_json::Value::Null)], cx);
                view.decide("one", "approve_once", cx);
            });
            assert!(global.read(cx).requests.borrow()["one"].busy);
            global.update(cx, |view, cx| view.set_request_state("one", false, Some("Try again".into()), cx));
            assert_eq!(scoped.read(cx).requests.borrow()["one"].error.as_deref(), Some("Try again"));
            scoped.update(cx, |view, cx| view.set_entries(Vec::new(), cx));
            assert!(global.read(cx).requests.borrow().contains_key("one"), "filter changes cannot erase global request state");
            global.update(cx, |view, cx| view.set_entries(Vec::new(), cx));
            assert!(scoped.read(cx).requests.borrow().is_empty(), "authoritative removal clears shared state");
        });
    }

    #[gpui_kit::test]
    fn notifications_retained_snapshot_guards_actions_and_preserves_open_popup(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let mut bell = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(NotificationBell::new);
            bell = Some(view.clone());
            gpui_kit::component::Root::new(view, window, cx)
        });
        let bell = bell.unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        visual.update(|_, cx| {
            let captured = events.clone();
            cx.subscribe(&bell, move |_, event: &NotificationEvent, _| {
                captured.borrow_mut().push(event.clone())
            })
            .detach();
            bell.update(cx, |view, cx| {
                assert!(!view.has_notifications());
                view.open = true;
                view.set_entries(vec![approval("one", serde_json::Value::Null)], cx);
                assert!(view.has_notifications());
            });
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let action = visual
            .debug_bounds("notification-approve_once-one")
            .expect("inline action rendered");
        visual.simulate_click(action.center(), Modifiers::none());
        visual.update(|_, cx| bell.update(cx, |view, cx| view.decide("one", "deny", cx)));
        visual.run_until_parked();
        assert_eq!(events.borrow().len(), 1);
        assert!(
            matches!(&events.borrow()[0], NotificationEvent::Decide { approval_id, decision, .. } if approval_id == "one" && decision == "approve_once")
        );
        visual.update(|window, cx| {
            bell.update(cx, |view, cx| {
                view.set_entries(
                    vec![approval("form", serde_json::json!({"kind":"text"}))],
                    cx,
                );
                assert!(view.open);
                view.decide("one", "deny", cx); // stale request ID
                view.decide("form", "approve_once", cx); // forms require review
                assert!(!view.requests.borrow().contains_key("one"));
            });
            window.draw(cx).clear(cx);
        });
        visual.run_until_parked();
        assert_eq!(events.borrow().len(), 1);
        assert!(visual.debug_bounds("notification-list").is_some());
        visual.update(|_, cx| {
            bell.update(cx, |view, cx| {
                view.set_entries(Vec::new(), cx);
                assert!(!view.has_notifications());
                assert!(!view.open);
            })
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let trigger = visual.debug_bounds("notification-trigger").unwrap();
        visual.simulate_click(trigger.center(), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("notification-list").is_none());
        visual.update(|_, cx| bell.update(cx, |view, _| assert!(!view.open)));
    }
    #[gpui_kit::test]
    fn notifications_final_dismiss_closes_immediately(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            let bell = cx.new(NotificationBell::new);
            bell.update(cx, |view, cx| {
                let mut update = approval("last", serde_json::Value::Null);
                update.kind = NotificationKind::Message;
                view.set_entries(vec![update], cx);
                view.open = true;
                view.dismiss("last", cx);
                assert!(
                    !view.open,
                    "dismiss closes before the owner publishes updated entries"
                );
            });
        });
    }
    #[gpui_kit::test]
    fn notifications_rows_expand_without_navigation_and_move_actions_under_summary(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let mut bell = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(NotificationBell::new);
            bell = Some(view.clone());
            gpui_kit::component::Root::new(view, window, cx)
        });
        let bell = bell.unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        visual.update(|_, cx| {
            let events = events.clone();
            cx.subscribe(&bell, move |_, event: &NotificationEvent, _| {
                events.borrow_mut().push(event.clone())
            })
            .detach();
            bell.update(cx, |view, cx| {
                let mut request = approval("expand", serde_json::Value::Null);
                request.detail =
                    "This is a longer summary with enough detail to need an expanded view. "
                        .repeat(5);
                let mut transient = approval("update", serde_json::Value::Null);
                transient.kind = NotificationKind::Attention;
                transient.tone = NotificationTone::Success;
                view.open = true;
                view.set_entries(vec![request, transient], cx);
            });
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let list = visual.debug_bounds("notification-list").unwrap();
        let row = visual.debug_bounds("notification-row-expand").unwrap();
        assert_eq!(row.left(), list.left(), "rows reach the list edge");
        assert_eq!(row.right(), list.right(), "no inset card gutter");
        let approve = visual
            .debug_bounds("notification-approve_once-expand")
            .unwrap();
        let deny = visual.debug_bounds("notification-deny-expand").unwrap();
        assert_eq!(
            approve.left(),
            deny.left(),
            "collapsed actions share a vertical rail"
        );
        assert!(approve.bottom() <= deny.top());
        assert!(visual.debug_bounds("notification-count").is_some());
        assert!(
            visual.debug_bounds("notification-expand-update").is_none(),
            "short updates do not show an expand control"
        );
        let expand = visual.debug_bounds("notification-expand-expand").unwrap();
        assert!(
            expand.right() < approve.left(),
            "expand lives in the left icon column"
        );
        assert_eq!(
            expand.bottom(),
            row.bottom() - gpui_kit::px(10.) - gpui_kit::px(1.),
            "expand sits at the row bottom"
        );
        visual.simulate_click(expand.center(), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            events.borrow().is_empty(),
            "expand must not navigate or approve"
        );
        let approve = visual
            .debug_bounds("notification-approve_once-expand")
            .unwrap();
        let deny = visual.debug_bounds("notification-deny-expand").unwrap();
        assert_eq!(
            approve.top(),
            deny.top(),
            "expanded actions become a horizontal button row"
        );
        assert!(
            approve.size.width > gpui_kit::px(26.),
            "expanded actions have text labels"
        );
        visual.update(|_, cx| {
            bell.update(cx, |view, cx| {
                let entries = view.entries.clone();
                view.set_entries(entries, cx);
                assert!(view.expanded.contains("expand"));
                view.dismiss("expand", cx); // fixed requests cannot be discarded
            })
        });
        assert!(events.borrow().is_empty());
        let dismiss = visual.debug_bounds("notification-dismiss-update").unwrap();
        visual.simulate_click(dismiss.center(), Modifiers::none());
        visual.run_until_parked();
        assert_eq!(
            *events.borrow(),
            vec![NotificationEvent::Dismiss {
                id: "update".into()
            }]
        );
        let row = visual.debug_bounds("notification-row-expand").unwrap();
        visual.simulate_click(
            gpui_kit::point(
                row.left() + gpui_kit::px(60.),
                row.top() + gpui_kit::px(16.),
            ),
            Modifiers::none(),
        );
        visual.run_until_parked();
        assert!(
            matches!(events.borrow().last(),Some(NotificationEvent::Review {task_id,request_id:None}) if task_id=="task")
        );
        visual.update(|_, cx| {
            bell.update(cx, |view, cx| {
                assert!(!view.open);
                view.set_entries(Vec::new(), cx);
                assert!(view.expanded.is_empty());
            })
        });
    }
}
