//! Lightweight native activity graphics shared by chats, tabs, and sidebar rows.
use crate::design::{self, Palette};
use gpui_kit::{prelude::*, *};
use std::time::Duration;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ActivityAnimation {
    #[default]
    Sparkles,
    Grid,
    Matrix,
}

impl ActivityAnimation {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "grid" => Self::Grid,
            "matrix" => Self::Matrix,
            _ => Self::Sparkles,
        }
    }
}

/// A small, persistent run marker. It is visible even with Reduce Motion on.
pub fn marker(
    animation: ActivityAnimation,
    palette: Palette,
    reduced_motion: bool,
    id: impl Into<SharedString>,
) -> AnyElement {
    let glyph = match animation {
        ActivityAnimation::Sparkles => "✦",
        ActivityAnimation::Grid => "▦",
        ActivityAnimation::Matrix => "▤",
    };
    let mark = div()
        .id(id)
        .flex_shrink_0()
        .text_size(design::ui_scaled_px(11.))
        .text_color(rgb(palette.accent))
        .child(glyph);
    if reduced_motion {
        mark.into_any_element()
    } else {
        mark.with_animation(
            "activity-marker",
            Animation::new(Duration::from_millis(1800)).repeat(),
            |mark, progress| mark.opacity(0.42 + 0.58 * (1. - (2. * progress - 1.).abs())),
        )
        .into_any_element()
    }
}

/// Ambient pane graphic, placed behind the transcript and composer.
pub fn pane(
    animation: ActivityAnimation,
    palette: Palette,
    reduced_motion: bool,
) -> AnyElement {
    let glyph = match animation {
        ActivityAnimation::Sparkles => "✦",
        ActivityAnimation::Grid => "╋",
        ActivityAnimation::Matrix => "01",
    };
    let mut field = div()
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .h(design::ui_scaled_px(180.))
        .overflow_hidden()
        .flex()
        .justify_around()
        .items_end();
    for ix in 0..7 {
        let symbol = div()
            .id(SharedString::from(format!("pane-activity-{ix}")))
            .text_size(design::ui_scaled_px(if ix % 3 == 0 { 14. } else { 10. }))
            .font_family(design::FONT_MONO)
            .text_color(rgb(palette.accent))
            .opacity(if reduced_motion { 0.12 } else { 0.22 })
            .child(glyph);
        field = field.child(if reduced_motion {
            symbol.into_any_element()
        } else {
            symbol
                .with_animation(
                    SharedString::from(format!("pane-activity-motion-{ix}")),
                    Animation::new(Duration::from_millis(2400 + ix as u64 * 270)).repeat(),
                    |symbol, progress| {
                        symbol
                            .opacity(0.07 + 0.21 * (1. - (2. * progress - 1.).abs()))
                            .bottom(design::ui_scaled_px(progress * 85.))
                    },
                )
                .into_any_element()
        });
    }
    field.into_any_element()
}
