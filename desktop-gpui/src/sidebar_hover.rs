//! Event-driven retained hover state and a short, keyed row-background fade.
use super::*;
use gpui_kit::base::motion::{MotionStatus, Transition, transition_with_status};
use std::time::Duration;

pub(super) struct HoverState {
    hovered: bool,
}

pub(super) fn state<V: 'static>(
    key: &str,
    selected: bool,
    window: &mut Window,
    cx: &mut Context<V>,
) -> (Entity<HoverState>, f32) {
    let state = window.use_keyed_state(
        SharedString::from(format!("sidebar-hover-{key}")),
        cx,
        |_, _| HoverState { hovered: false },
    );
    let visible = selected || state.read(cx).hovered;
    let motion = transition_with_status(
        SharedString::from(format!("sidebar-hover-opacity-{key}")),
        if visible { 1.0 } else { 0.0 },
        Transition::new(Duration::from_millis(180)),
        window,
        cx,
    );
    if matches!(motion.status, MotionStatus::Running | MotionStatus::Delayed) {
        // The keyed transition requests frames, but retained panel content can
        // replay a cached row unless its owning Desktop entity is invalidated.
        // Keep that notification frame-local and stop as soon as the fade ends.
        let owner = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            let _ = owner.update(cx, |_, cx| cx.notify());
        });
    }
    (state, motion.value)
}

pub(super) fn set<V: 'static>(state: &Entity<HoverState>, hovered: bool, cx: &mut Context<V>) {
    let changed = state.update(cx, |state, _| {
        if state.hovered == hovered {
            false
        } else {
            state.hovered = hovered;
            true
        }
    });
    if changed {
        cx.notify();
    }
}
