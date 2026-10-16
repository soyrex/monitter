//! Small reversible hover color transitions for retained components.
use gpui::{App, Background, ElementId, Entity, Hsla, SharedString, Window};
use gpui_base::motion::{MotionStatus, Transition, transition_with_status};
use std::time::Duration;

const DURATION: Duration = Duration::from_millis(180);

struct HoverState {
    hovered: bool,
}

/// Retained hover state shared by a component's background and foreground fades.
/// IDs must be stable and unique among sibling components.
#[derive(Clone)]
pub struct HoverMotion {
    state: Entity<HoverState>,
    enabled: bool,
    id: SharedString,
}

impl HoverMotion {
    pub fn new(id: impl Into<ElementId>, enabled: bool, window: &mut Window, cx: &mut App) -> Self {
        let id = id.into();
        let key = SharedString::from(format!("hover-motion-state-{id}"));
        let state = window.use_keyed_state(key, cx, |_, _| HoverState { hovered: false });
        if !enabled && state.read(cx).hovered {
            state.update(cx, |state, cx| {
                state.hovered = false;
                cx.notify();
            });
        }
        Self {
            state,
            enabled,
            id: SharedString::from(id.to_string()),
        }
    }

    /// Interpolate one color channel toward its hover target.
    pub fn color(
        &self,
        channel: impl Into<SharedString>,
        normal: Hsla,
        hovered: Hsla,
        window: &mut Window,
        cx: &mut App,
    ) -> Hsla {
        let target = if self.enabled && self.state.read(cx).hovered {
            hovered
        } else {
            normal
        };
        let channel = channel.into();
        let key = SharedString::from(format!("hover-motion-color-{}-{channel}", self.id));
        let motion = transition_with_status(key, target, Transition::new(DURATION), window, cx);
        if matches!(motion.status, MotionStatus::Running | MotionStatus::Delayed) {
            window.request_animation_frame();
        }
        motion.value
    }

    /// Interpolate solid backgrounds while preserving their blend mode. Non-solid
    /// backgrounds retain their original gradient or pattern and switch as a unit.
    pub fn background(
        &self,
        channel: impl Into<SharedString>,
        normal: Background,
        hovered: Background,
        window: &mut Window,
        cx: &mut App,
    ) -> Background {
        let (Some(normal_color), Some(hover_color)) = (normal.as_solid(), hovered.as_solid())
        else {
            return if self.enabled && self.state.read(cx).hovered {
                hovered
            } else {
                normal
            };
        };
        let mut result: Background = self
            .color(channel, normal_color, hover_color, window, cx)
            .into();
        result.blend_mode = normal.blend_mode;
        result
    }

    /// Update the hover target from an element's enter/leave callback.
    pub fn on_hover(&self, hovered: bool, cx: &mut App) {
        if !self.enabled {
            return;
        }
        self.state.update(cx, |state, cx| {
            if state.hovered != hovered {
                state.hovered = hovered;
                cx.notify();
            }
        });
    }
}
