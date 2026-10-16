//! One retained Copy control per semantic message; expiry never notifies the chat pane.
use super::feedback::{self, Clock, Confirmation, FeedbackClock};
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    assets::IconName,
    component::{Disableable, Icon, Sizable, button::*},
    prelude::*,
    *,
};
use std::{rc::Rc, time::Duration};

pub(super) struct CopyFeedback {
    message_id: String,
    copy: Rc<dyn Fn(&mut App) -> bool>,
    disabled: bool,
    visible: bool,
    confirmation: Confirmation,
    expiry: Option<Task<()>>,
    clock: Clock,
}
impl CopyFeedback {
    pub(super) fn new(message_id: String, copy: Rc<dyn Fn(&mut App) -> bool>) -> Self {
        Self {
            message_id,
            copy,
            disabled: false,
            visible: true,
            confirmation: Confirmation::default(),
            expiry: None,
            clock: FeedbackClock::new(),
        }
    }
    pub(super) fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
    }
    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.clock.borrow_mut().set_visible(visible);
        if !visible {
            self.expiry = None;
            let generation = self.confirmation.renew();
            self.confirmation.expire(generation);
        }
        cx.notify();
    }
    fn confirm(&mut self, cx: &mut Context<Self>) {
        let already_confirmed = self.confirmation.confirmed();
        let generation = self.confirmation.renew();
        if !already_confirmed {
            self.clock.borrow_mut().start(Duration::from_millis(100));
        }
        let change_ms = if !already_confirmed
            && !cx.reduce_motion()
            && crate::native_motion::motion_allowed()
        {
            100
        } else {
            0
        };
        self.expiry = Some(cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(change_ms + 800))
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.confirmation.expire(generation) {
                    view.expiry = None;
                    if view.visible {
                        view.clock.borrow_mut().start(Duration::from_millis(100));
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }
}
impl Render for CopyFeedback {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self
            .clock
            .borrow_mut()
            .progress(cx.reduce_motion() || !crate::native_motion::motion_allowed());
        let check = if self.confirmation.confirmed() {
            progress
        } else {
            1. - progress
        };
        let label = if self.confirmation.confirmed() {
            "Copied message text"
        } else {
            "Copy message text"
        };
        feedback::painted(
            Button::new(SharedString::from(format!(
                "copy-message-{}",
                self.message_id
            )))
            .ghost()
            .compact()
            .w(px(23.))
            .h(px(23.))
            .p_0()
            .child(
                div()
                    .relative()
                    .size(px(13.))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .opacity(1. - check)
                            .child(Icon::new(IconName::Copy).size(px(13.))),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .opacity(check)
                            .child(Icon::new(IconName::Check).size(px(13.))),
                    ),
            )
            .disabled(self.disabled)
            .accessibility_label(label)
            .tooltip(label)
            .on_click(cx.listener(|view, _, _, cx| {
                if (view.copy)(cx) {
                    view.confirm(cx);
                }
            })),
            self.clock.clone(),
        )
    }
}
