//! Local completion markers seeded from authoritative records, never mount effects.
use crate::chat::feedback::{self, Clock, FeedbackClock};
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::{Sizable, spinner::Spinner},
    prelude::*,
    *,
};
use std::time::Duration;

pub(crate) struct CompletionState {
    id: String,
    status: String,
}
impl CompletionState {
    fn observe(&mut self, id: &str, status: &str, live_visible: bool) -> bool {
        let complete =
            self.id == id && self.status == "running" && status == "completed" && live_visible;
        if self.id != id {
            self.id = id.to_owned();
        }
        if self.status != status {
            self.status = status.to_owned();
        }
        complete
    }
}
pub(crate) struct CompletionMarker {
    state: CompletionState,
    clock: Clock,
    icon: &'static str,
    color: u32,
    size: f32,
    visible: bool,
}
impl CompletionMarker {
    pub(crate) fn new(id: &str, status: &str) -> Self {
        Self {
            state: CompletionState {
                id: id.to_owned(),
                status: status.to_owned(),
            },
            clock: FeedbackClock::new(),
            icon: "",
            color: 0,
            size: 12.,
            visible: true,
        }
    }
    pub(crate) fn set_status(
        &mut self,
        id: &str,
        status: &str,
        live: bool,
        cx: &mut Context<Self>,
    ) {
        let visible = self.clock.borrow().painted;
        let changed = self.state.id != id || self.state.status != status;
        if self.state.observe(id, status, live && visible) {
            self.clock.borrow_mut().start(Duration::from_millis(160));
        } else if status != "completed" || !live {
            self.clock.borrow_mut().finish();
        }
        if changed {
            // A published running state must actually paint before a later success
            // can animate; two updates in one frame cannot invent that observation.
            self.clock.borrow_mut().painted = false;
            cx.notify();
        }
    }
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.clock.borrow_mut().set_visible(visible);
        cx.notify();
    }
    pub(crate) fn suppress(&mut self, cx: &mut Context<Self>) {
        self.clock.borrow_mut().finish();
        self.clock.borrow_mut().painted = false;
        cx.notify();
    }
    pub(crate) fn set_appearance(
        &mut self,
        icon: &'static str,
        color: u32,
        size: f32,
        cx: &mut Context<Self>,
    ) {
        if (self.icon, self.color, self.size) != (icon, color, size) {
            self.icon = icon;
            self.color = color;
            self.size = size;
            cx.notify();
        }
    }
}
impl Render for CompletionMarker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self
            .clock
            .borrow_mut()
            .progress(cx.reduce_motion() || !crate::native_motion::motion_allowed());
        let marker = div()
            .flex_none()
            .flex()
            .items_center()
            .text_size(px(self.size))
            .text_color(rgb(self.color));
        let marker = if self.state.status == "running" {
            marker.child(Spinner::new().xsmall().color(rgb(self.color).into()))
        } else {
            marker.child(div().opacity(progress).child(self.icon))
        };
        feedback::painted(marker, self.clock.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state(id: &str, status: &str) -> CompletionState {
        CompletionState {
            id: id.into(),
            status: status.into(),
        }
    }
    #[::core::prelude::v1::test]
    fn only_visible_observed_success_animates_once() {
        let mut state = state("a", "running");
        assert!(state.observe("a", "completed", true));
        assert!(!state.observe("a", "completed", true));
        for final_status in ["error", "interrupted", "cancelled", "idle"] {
            assert!(!super::tests::state("a", "running").observe("a", final_status, true));
        }
    }
    #[::core::prelude::v1::test]
    fn initial_history_recycling_hidden_and_reconnect_are_static() {
        assert!(!state("a", "completed").observe("a", "completed", true));
        assert!(!state("a", "running").observe("b", "completed", true));
        let mut state = state("a", "running");
        assert!(!state.observe("a", "completed", false));
        assert!(!state.observe("a", "completed", true));
    }
}
