//! Native landing actions for a new workspace pane.
//!
//! This view emits intent only. The workspace owner decides whether a chat,
//! terminal, or browser can be opened and replaces the empty pane when that
//! operation succeeds.

use crate::{design, design::ui_scaled_px as px};
use gpui_kit::{
    assets::IconName,
    component::{Disableable, Icon, button::*},
    prelude::*,
    *,
};
use serde_json::Value;

const DEFAULT_FONT_RATIO: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyPaneAction {
    Chat,
    Terminal,
    Browser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyPaneEvent {
    Action(EmptyPaneAction),
}

impl EventEmitter<EmptyPaneEvent> for EmptyPane {}

pub struct EmptyPane {
    id: String,
    focus: FocusHandle,
    palette: Option<design::Palette>,
    interface_font_ratio: f32,
    busy: bool,
    browser_available: bool,
}

impl EmptyPane {
    pub fn new(id: impl Into<String>, cx: &mut Context<Self>) -> Self {
        Self {
            id: id.into(),
            focus: cx.focus_handle(),
            palette: None,
            interface_font_ratio: DEFAULT_FONT_RATIO,
            busy: false,
            browser_available: false,
        }
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

        div()
            .id(SharedString::from(format!("native-empty-pane-{}", self.id)))
            .size_full()
            .min_size_0()
            .overflow_y_scroll()
            .flex()
            .items_center()
            .justify_center()
            .p(px(20.0))
            .track_focus(&self.focus)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(16.0))
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
    Button::new(id)
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
                .items_center()
                .rounded(px(10.0))
                .p(px(20.0))
                .px(px(24.0))
                .gap(px(12.0))
                .child(Icon::new(icon).size(px(22.0)))
                .child(div().text_size(font_size).child(label)),
        )
        .into_any_element()
}
