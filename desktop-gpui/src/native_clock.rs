//! Immediate-mode sidebar clock. Draws the same geometry as the former SVG
//! clock while avoiding a new asynchronous image asset on every second.

use crate::design::{self, Palette};
use chrono::Timelike as _;
use gpui_kit::{PathBuilder, canvas, point, px, rgba, *};

const VIEWBOX: f32 = 60.0;
const RIM_RADIUS: f32 = 27.5;
const PIN_RADIUS: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ClockAngles {
    hour: f32,
    minute: f32,
    second: f32,
}

/// Convert local wall-clock components to the SVG-equivalent clockwise angles.
fn clock_angles(hour: u32, minute: u32, second: u32) -> ClockAngles {
    ClockAngles {
        hour: ((hour % 12) as f32 + minute as f32 / 60.0) * 30.0,
        minute: (minute as f32 + second as f32 / 60.0) * 6.0,
        second: second as f32 * 6.0,
    }
}

/// Build a clock at the owner's requested design-pixel diameter and time.
/// The current local timezone is used, matching `chrono::Local` in the former
/// SVG implementation. The clock itself has no timer; its owner supplies ticks.
pub fn clock_face(now_ms: u64, palette: Palette, size: f32) -> AnyElement {
    let timestamp = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let time = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp)
        .map(|utc| utc.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now);
    let angles = clock_angles(time.hour(), time.minute(), time.second());

    canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let scale = f32::from(bounds.size.width) / VIEWBOX;
            let origin = bounds.origin;
            let p = |x: f32, y: f32| {
                point(
                    px(f32::from(origin.x) + x * scale),
                    px(f32::from(origin.y) + y * scale),
                )
            };
            let color = |value: u32, alpha: u8| rgba((value << 8) | u32::from(alpha));

            // SVG rim: translucent panel fill with a 40%-opacity line stroke.
            if let Some(path) = circle_path(p, 30.0, 30.0, RIM_RADIUS, true, px(0.0)) {
                window.paint_path(path, color(palette.panel, 20));
            }
            if let Some(path) = circle_path(p, 30.0, 30.0, RIM_RADIUS, false, px(scale)) {
                window.paint_path(path, color(palette.line, 102));
            }

            // Twelve ticks; quarter-hour marks are longer and heavier.
            for index in 0..12 {
                let quarter = index % 3 == 0;
                let inner_y = if quarter { 10.0 } else { 8.0 };
                let rotation = index as f32 * 30.0;
                paint_segment(
                    window,
                    p,
                    radial_point(30.0 - 5.0, rotation),
                    radial_point(30.0 - inner_y, rotation),
                    if quarter { 1.5 } else { 1.0 },
                    scale,
                    color(palette.muted, 255),
                    false,
                );
            }

            // SVG hand order and values are preserved: hour, minute, second.
            paint_hand(
                window,
                p,
                31.0,
                18.0,
                angles.hour,
                2.7,
                color(palette.ink, 235),
            );
            paint_hand(
                window,
                p,
                32.0,
                11.0,
                angles.minute,
                1.8,
                color(palette.ink, 217),
            );
            paint_hand(
                window,
                p,
                34.0,
                9.0,
                angles.second,
                1.0,
                color(palette.accent, 242),
            );

            // The SVG pin is accent-filled, with a 60%-opacity line border.
            if let Some(path) = circle_path(p, 30.0, 30.0, PIN_RADIUS, true, px(0.0)) {
                window.paint_path(path, color(palette.accent, 255));
            }
            if let Some(path) = circle_path(p, 30.0, 30.0, PIN_RADIUS, false, px(scale)) {
                window.paint_path(path, color(palette.line, 153));
            }
        },
    )
    .w(design::ui_scaled_px(size))
    .h(design::ui_scaled_px(size))
    .into_any_element()
}

fn radial_point(radius: f32, degrees: f32) -> (f32, f32) {
    let radians = degrees.to_radians();
    (30.0 + radians.sin() * radius, 30.0 - radians.cos() * radius)
}

fn paint_hand(
    window: &mut Window,
    p: impl Fn(f32, f32) -> Point<Pixels> + Copy,
    inner: f32,
    outer: f32,
    angle: f32,
    width: f32,
    color: Rgba,
) {
    // SVG's hand constants are absolute y coordinates around y=30, not radii.
    let (x1, y1) = radial_point(30.0 - inner, angle);
    let (x2, y2) = radial_point(30.0 - outer, angle);
    let scale = (f32::from(p(1.0, 0.0).x) - f32::from(p(0.0, 0.0).x)).abs();
    paint_segment(window, p, (x1, y1), (x2, y2), width, scale, color, true);
}

fn paint_segment(
    window: &mut Window,
    p: impl Fn(f32, f32) -> Point<Pixels> + Copy,
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    scale: f32,
    color: Rgba,
    round_caps: bool,
) {
    let mut builder = PathBuilder::stroke(px(width * scale));
    builder.move_to(p(from.0, from.1));
    builder.line_to(p(to.0, to.1));
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }

    // GPUI's public facade exposes the lyon stroke options but not LineCap.
    // Small endpoint discs approximate SVG stroke-linecap="round" geometry.
    if round_caps {
        for (x, y) in [from, to] {
            if let Some(path) = circle_path(p, x, y, width / 2.0, true, px(0.0)) {
                window.paint_path(path, color);
            }
        }
    }
}

fn circle_path(
    p: impl Fn(f32, f32) -> Point<Pixels>,
    center_x: f32,
    center_y: f32,
    radius: f32,
    filled: bool,
    stroke_width: Pixels,
) -> Option<gpui_kit::Path<Pixels>> {
    let mut builder = if filled {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(stroke_width)
    };
    // Four cubic arcs, kappa = 4*(sqrt(2)-1)/3, matching a circle closely.
    let k = radius * 0.552_284_8;
    builder.move_to(p(center_x + radius, center_y));
    builder.cubic_bezier_to(
        p(center_x, center_y + radius),
        p(center_x + radius, center_y + k),
        p(center_x + k, center_y + radius),
    );
    builder.cubic_bezier_to(
        p(center_x - radius, center_y),
        p(center_x - k, center_y + radius),
        p(center_x - radius, center_y + k),
    );
    builder.cubic_bezier_to(
        p(center_x, center_y - radius),
        p(center_x - radius, center_y - k),
        p(center_x - k, center_y - radius),
    );
    builder.cubic_bezier_to(
        p(center_x + radius, center_y),
        p(center_x + k, center_y - radius),
        p(center_x + radius, center_y - k),
    );
    builder.close();
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::{clock_angles, radial_point};

    #[::core::prelude::v1::test]
    fn midnight_and_noon_share_the_twelve_oclock_hand_angles() {
        assert_eq!(clock_angles(0, 0, 0), clock_angles(12, 0, 0));
        assert_eq!(clock_angles(0, 0, 0).hour, 0.0);
    }

    #[::core::prelude::v1::test]
    fn quarter_past_advances_hour_and_minute_hands() {
        let angles = clock_angles(3, 15, 0);
        assert_eq!(angles.hour, 97.5);
        assert_eq!(angles.minute, 90.0);
        assert_eq!(angles.second, 0.0);
    }

    #[::core::prelude::v1::test]
    fn seconds_advance_minute_and_second_hands_fractionally() {
        let angles = clock_angles(9, 30, 15);
        assert_eq!(angles.hour, 285.0);
        assert_eq!(angles.minute, 181.5);
        assert_eq!(angles.second, 90.0);
    }

    #[::core::prelude::v1::test]
    fn midnight_hour_hand_matches_svg_tail_and_tip_coordinates() {
        let tail = radial_point(30.0 - 31.0, 0.0);
        let tip = radial_point(30.0 - 18.0, 0.0);
        assert!((tail.0 - 30.0).abs() < 0.001);
        assert!((tail.1 - 31.0).abs() < 0.001);
        assert!((tip.0 - 30.0).abs() < 0.001);
        assert!((tip.1 - 18.0).abs() < 0.001);
    }

    #[::core::prelude::v1::test]
    fn quarter_turn_rotates_hour_hand_endpoints_clockwise() {
        let tail = radial_point(30.0 - 31.0, 90.0);
        let tip = radial_point(30.0 - 18.0, 90.0);
        assert!((tail.0 - 29.0).abs() < 0.001);
        assert!((tail.1 - 30.0).abs() < 0.001);
        assert!((tip.0 - 42.0).abs() < 0.001);
        assert!((tip.1 - 30.0).abs() < 0.001);
    }
}
