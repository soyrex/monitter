//! Native landing actions for a new workspace pane.
//!
//! This view emits intent only. The workspace owner decides whether a chat,
//! terminal, or browser can be opened and replaces the empty pane when that
//! operation succeeds.

use crate::{design, design::ui_scaled_px as px};
use gpui_kit::{
    assets::IconName,
    component::{
        Disableable, Icon,
        button::*,
        input::{Input, InputEvent, InputState},
    },
    prelude::*,
    *,
};
use monitter_gpui_desktop::session_picker::{PickerGroups, SessionEntry, matches_query};
use serde_json::Value;

const DEFAULT_FONT_RATIO: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyPaneAction {
    Chat,
    Terminal,
    Browser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmptyPaneEvent {
    Action(EmptyPaneAction),
    OpenSession(String),
}
#[derive(Clone)]
enum PickerRow {
    Heading(&'static str),
    Session(SessionEntry),
}

impl EventEmitter<EmptyPaneEvent> for EmptyPane {}

pub struct EmptyPane {
    id: String,
    focus: FocusHandle,
    palette: Option<design::Palette>,
    interface_font_ratio: f32,
    busy: bool,
    browser_available: bool,
    query: Entity<InputState>,
    _query_change: Subscription,
    sessions: PickerGroups,
    rows: Vec<PickerRow>,
    selected: Option<String>,
    scroll: UniformListScrollHandle,
}

impl EmptyPane {
    pub fn new(id: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search all unarchived chats…"));
        let query_change =
            cx.subscribe_in(&query, window, |pane, _, event, window, cx| match event {
                InputEvent::Change => pane.refilter(true, cx),
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => {
                    if !pane.query.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    }) {
                        pane.confirm(cx);
                    }
                }
                _ => {}
            });
        Self {
            id: id.into(),
            focus: cx.focus_handle(),
            palette: None,
            interface_font_ratio: DEFAULT_FONT_RATIO,
            busy: false,
            browser_available: false,
            query,
            _query_change: query_change,
            sessions: PickerGroups::default(),
            rows: vec![],
            selected: None,
            scroll: UniformListScrollHandle::default(),
        }
    }
    pub fn set_sessions(&mut self, sessions: &PickerGroups, cx: &mut Context<Self>) {
        if &self.sessions != sessions {
            self.sessions = sessions.clone();
            self.refilter(false, cx);
        }
    }
    fn refilter(&mut self, reveal: bool, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        self.rows.clear();
        for (label, entries) in [
            ("Running", &self.sessions.running),
            ("Recent", &self.sessions.recent),
        ] {
            let matches: Vec<_> = entries
                .iter()
                .filter(|entry| matches_query(entry, &query))
                .cloned()
                .collect();
            if !matches.is_empty() {
                self.rows.push(PickerRow::Heading(label));
                self.rows
                    .extend(matches.into_iter().map(PickerRow::Session));
            }
        }
        if reveal || !self.rows.iter().any(|row| matches!(row, PickerRow::Session(entry) if Some(&entry.id) == self.selected.as_ref())) {
            self.selected = self.rows.iter().find_map(|row| match row { PickerRow::Session(entry) => Some(entry.id.clone()), _ => None });
        }
        if reveal {
            self.reveal_selection();
        }
        cx.notify();
    }
    fn reveal_selection(&self) {
        if let Some(index) = self.rows.iter().position(|row| matches!(row, PickerRow::Session(entry) if Some(&entry.id) == self.selected.as_ref())) {
            self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        }
    }
    fn confirm(&self, cx: &mut Context<Self>) {
        if !self.busy {
            if let Some(id) = &self.selected {
                cx.emit(EmptyPaneEvent::OpenSession(id.clone()));
            }
        }
    }
    fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        let ids: Vec<_> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                PickerRow::Session(entry) => Some(entry.id.clone()),
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let current = ids
            .iter()
            .position(|id| Some(id) == self.selected.as_ref())
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % ids.len()
        } else {
            (current + ids.len() - 1) % ids.len()
        };
        self.selected = Some(ids[next].clone());
        self.reveal_selection();
        cx.notify();
    }

    /// Update presentation and owner-provided action availability.
    pub fn set_presentation(
        &mut self,
        palette: design::Palette,
        interface_font_ratio: f32,
        busy: bool,
        browser_available: bool,
        cx: &mut Context<Self>,
    ) {
        let ratio = normalize_font_ratio(interface_font_ratio);
        if self.palette == Some(palette)
            && self.interface_font_ratio == ratio
            && self.busy == busy
            && self.browser_available == browser_available
        {
            return;
        }
        self.palette = Some(palette);
        self.interface_font_ratio = ratio;
        self.busy = busy;
        self.browser_available = browser_available;
        cx.notify();
    }
}

impl Focusable for EmptyPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for EmptyPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| design::palette(&Value::Null, window.appearance()));
        let font_size = px(14.0 * self.interface_font_ratio);
        let busy = self.busy;
        let browser_available = self.browser_available;
        let card_style = ButtonCustomVariant::new(cx)
            .color(crate::design::surface_color(palette.panel).into())
            .foreground(rgb(palette.muted).into())
            .hover(crate::design::surface_color(palette.soft).into())
            .active(crate::design::surface_color(palette.soft).into());
        let rows = if self.rows.is_empty() {
            vec![PickerRow::Heading(
                if self.query.read(cx).value().trim().is_empty() {
                    "No unarchived chats yet."
                } else {
                    "No matching chats."
                },
            )]
        } else {
            self.rows.clone()
        };
        let owner = cx.weak_entity();
        let selected = self.selected.clone();
        let list = uniform_list(
            SharedString::from(format!("{}-session-results", self.id)),
            rows.len(),
            move |range, _, _| {
                range
                    .map(|index| match &rows[index] {
                        PickerRow::Heading(label) => div()
                            .w_full()
                            .h(px(48.))
                            .px_3()
                            .flex()
                            .items_center()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(11.))
                            .text_color(rgb(palette.muted))
                            .child(*label)
                            .into_any_element(),
                        PickerRow::Session(entry) => {
                            let id = entry.id.clone();
                            let owner = owner.clone();
                            let metadata = [entry.agent_name.as_str(), entry.project_name.as_str()]
                                .into_iter()
                                .filter(|text| !text.is_empty())
                                .collect::<Vec<_>>()
                                .join(" · ");
                            div()
                                .id(SharedString::from(format!("existing-session-{id}")))
                                .w_full()
                                .role(Role::Button)
                                .aria_label(format!(
                                    "Open existing chat: {} · {metadata}",
                                    entry.title
                                ))
                                .h(px(48.))
                                .px_3()
                                .flex()
                                .items_center()
                                .gap_2()
                                .rounded_md()
                                .when(selected.as_ref() == Some(&entry.id), |row| {
                                    row.bg(design::surface_color(palette.soft))
                                })
                                .hover(|row| row.bg(design::surface_color(palette.soft)))
                                .cursor_pointer()
                                .text_color(rgb(palette.ink))
                                .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(
                                    if entry.status == "running" {
                                        palette.accent
                                    } else {
                                        palette.muted
                                    },
                                )))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(px(13.))
                                                .child(entry.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(px(10.))
                                                .text_color(rgb(palette.muted))
                                                .child(metadata),
                                        ),
                                )
                                .when_some(
                                    crate::timestamps::local_datetime(entry.updated_at)
                                        .filter(|_| entry.updated_at > 0),
                                    |row, at| {
                                        row.child(
                                            div()
                                                .debug_selector({ let id = id.clone(); move || format!("empty-pane-date-{id}").into() })
                                                .w(px(56.))
                                                .flex_shrink_0()
                                                .text_right()
                                                .whitespace_nowrap()
                                                .text_size(px(10.))
                                                .text_color(rgb(palette.muted))
                                                .child(at.format("%b %-d").to_string()),
                                        )
                                    },
                                )
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |pane, cx| {
                                        pane.selected = Some(id.clone());
                                        pane.confirm(cx);
                                    });
                                })
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            },
        )
        .debug_selector({ let id = self.id.clone(); move || format!("empty-pane-results-{id}").into() })
        .w_full()
        .flex_1()
        .min_h_0()
        .track_scroll(&self.scroll);
        let list = ScrollEdgeFade::new(
            list,
            self.scroll.0.borrow().base_handle.clone(),
            Axis::Vertical,
            gpui_kit::px(20.),
        );
        let picker = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .mt(px(20.))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child("Open an existing chat"),
            )
            .child(
                div().w_full().flex_shrink_0().child(
                    Input::new(&self.query)
                        .aria_label("Search all unarchived chats")
                        .disabled(busy)),
            )
            .child(list);

        div()
            .id(SharedString::from(format!("native-empty-pane-{}", self.id)))
            .debug_selector({ let id = self.id.clone(); move || format!("native-empty-pane-{id}").into() })
            .size_full()
            .min_size_0()
            .overflow_hidden()
            .bg(rgba(if palette.dark { 0x0000001a } else { 0xffffff1a }))
            .flex()
            .flex_col()
            .items_center()
            .p(px(20.0))
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|pane, event: &KeyDownEvent, window, cx| {
                let modifiers = event.keystroke.modifiers;
                if modifiers.control || modifiers.platform || modifiers.alt || modifiers.shift {
                    return;
                }
                if pane.query.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                }) {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "up" | "down" => {
                        pane.move_selection(event.keystroke.key == "down", cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_shrink_0()
                            .gap(px(10.0))
                            .child(action_card(
                                format!("{}-new-chat", self.id),
                                "New chat",
                                IconName::MessageSquare,
                                busy,
                                palette,
                                card_style,
                                font_size,
                                cx.listener(|_, _, _, cx| {
                                    cx.emit(EmptyPaneEvent::Action(EmptyPaneAction::Chat));
                                }),
                            ))
                            .child(action_card(
                                format!("{}-new-terminal", self.id),
                                "Terminal",
                                IconName::SquareTerminal,
                                busy,
                                palette,
                                card_style,
                                font_size,
                                cx.listener(|_, _, _, cx| {
                                    cx.emit(EmptyPaneEvent::Action(EmptyPaneAction::Terminal));
                                }),
                            ))
                            .child(action_card(
                                format!("{}-new-browser", self.id),
                                "Browser",
                                IconName::Globe,
                                !browser_available,
                                palette,
                                card_style,
                                font_size,
                                cx.listener(|_, _, _, cx| {
                                    cx.emit(EmptyPaneEvent::Action(EmptyPaneAction::Browser));
                                }),
                            )),
                    )
                    .child(picker),
            )
    }
}

fn normalize_font_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(8.0 / 14.0, 32.0 / 14.0)
    } else {
        DEFAULT_FONT_RATIO
    }
}

fn action_card(
    id: String,
    label: &'static str,
    icon: IconName,
    disabled: bool,
    palette: design::Palette,
    button_style: ButtonCustomVariant,
    font_size: Pixels,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let group = SharedString::from(format!("empty-pane-card-{id}"));
    let selector = format!("empty-pane-action-{id}");
    Button::new(id)
        .debug_selector(move || selector.clone())
        .flex_1()
        .min_w_0()
        .group(group.clone())
        .h_auto()
        .p_0()
        .border_1()
        .border_color(rgb(palette.line))
        .bg(design::surface_color(palette.panel))
        .rounded(px(10.0))
        .group_hover(group.clone(), |style| {
            style
                .bg(crate::design::surface_color(palette.soft))
                .border_color(rgb(palette.accent))
                .text_color(rgb(palette.ink))
        })
        .text_size(font_size)
        .focus(|style| {
            style
                .bg(crate::design::surface_color(palette.soft))
                .border_color(rgb(palette.accent))
                .text_color(rgb(palette.ink))
        })
        .custom(button_style)
        .accessibility_label(label)
        .disabled(disabled)
        .on_click(on_click)
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .justify_center()
                .rounded(px(10.0))
                .py(px(14.0))
                .px(px(10.0))
                .gap(px(8.0))
                .child(Icon::new(icon).size(px(22.0)).flex_shrink_0())
                .child(div().min_w_0().truncate().text_size(font_size).child(label)),
        )
        .into_any_element()
}

#[cfg(test)]
mod layout_tests {
    use super::{EmptyPane, PickerGroups, SessionEntry};
    use gpui_kit::{AppContext as _, TestAppContext, px, size};

    #[gpui_kit::test]
    fn empty_pane_actions_stay_in_three_columns_and_results_fill_height(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut pane = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| EmptyPane::new("layout", window, cx));
            pane = Some(view.clone());
            gpui_kit::component::Root::new(view, window, cx)
        });
        let pane = pane.unwrap();
        let sessions = PickerGroups { running: vec![], recent: (0..30).map(|i| SessionEntry {
            id: format!("session-{i}"), title: if i == 0 { "A very long chat title that must truncate within its row".into() } else { "Chat".into() },
            agent_name: "Codex".into(), project_name: "Monitter".into(), search_text: "chat".into(),
            updated_at: 1_791_831_000_000 - i * 86_400_000, status: "idle".into(),
        }).collect() };
        cx.update(|_, cx| pane.update(cx, |pane, cx| pane.set_sessions(&sessions, cx)));
        let mut short_height = None;
        for (width, height) in [(320., 500.), (640., 800.)] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.run_until_parked(); cx.update(|window, cx| window.draw(cx).clear(cx));
            let actions: Vec<_> = ["empty-pane-action-layout-new-chat", "empty-pane-action-layout-new-terminal", "empty-pane-action-layout-new-browser"].into_iter()
                .map(|selector| cx.debug_bounds(selector)
                    .expect("action has native layout bounds")).collect();
            assert!((actions[0].top() - actions[2].top()).abs() < px(1.));
            assert!((actions[0].size.width - actions[2].size.width).abs() < px(1.));
            assert!(actions[0].right() < actions[1].left() && actions[1].right() < actions[2].left());
            let results = cx.debug_bounds("empty-pane-results-layout").unwrap();
            let root = cx.debug_bounds("native-empty-pane-layout").unwrap();
            assert!((results.bottom() - (root.bottom() - super::px(20.))).abs() < px(1.));
            let first_date = cx.debug_bounds("empty-pane-date-session-0").unwrap();
            let second_date = cx.debug_bounds("empty-pane-date-session-1").unwrap();
            assert!((first_date.right() - second_date.right()).abs() < px(1.));
            assert!(first_date.right() <= results.right());
            if let Some(previous) = short_height { assert!(results.size.height > previous + px(200.)); }
            else { short_height = Some(results.size.height); }
            // The list remains a scroll surface instead of growing the whole pane.
            let offset = cx.update(|_, cx| pane.read(cx).scroll.0.borrow().base_handle.max_offset().y);
            assert!(offset > px(0.));
        }
    }
}
