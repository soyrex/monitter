//! Pane-only decorative activity patterns. The retained SparkleField owns the
//! clock, visibility and cadence; these canvases never schedule a frame.
use crate::{activity_sparkles::PaintBounds, design::Palette};
use gpui_kit::*;
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanePattern {
    Liquid,
    Matrix,
    Grid,
}

/// Mount in the same lower-half container as the pane sparkle canvas. Explicit
/// paint bounds describe the whole pane and are reduced to its lower half here.
/// `None` phase is the faint, completely static reduced-motion presentation.
pub(crate) fn build_pattern(
    _id: SharedString,
    pattern: PanePattern,
    palette: Palette,
    motion_allowed: bool,
    phase: Option<f32>,
    paint_bounds: Option<PaintBounds>,
) -> AnyElement {
    let phase = if motion_allowed { phase } else { None };
    let unit = crate::design::interface_scale() as f32 / 100.;
    canvas(
        move |bounds, _, _| {
            if let Some(mut pane) = paint_bounds.as_ref().and_then(|bounds| bounds.get()) {
                pane.origin.y += pane.size.height * 0.5;
                pane.size.height *= 0.5;
                pane
            } else {
                bounds
            }
        },
        move |_, bounds, window, _| {
            if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
                return;
            }
            window.with_content_mask(Some(ContentMask::new(bounds)), |window| {
                let strength = if phase.is_some() { 1. } else { 0.35 };
                let phase = phase.unwrap_or(0.25);
                match pattern {
                    PanePattern::Liquid => liquid(window, bounds, palette.accent, phase, strength),
                    PanePattern::Matrix => {
                        matrix(window, bounds, palette.accent, phase, strength, unit)
                    }
                    PanePattern::Grid => {
                        grid(window, bounds, palette.accent, phase, strength, unit)
                    }
                }
            });
        },
    )
    .size_full()
    .into_any_element()
}

fn color(rgb: u32, alpha: f32) -> Rgba {
    rgba((rgb << 8) | (alpha.clamp(0., 255.).round() as u32))
}

fn rect(window: &mut Window, x: f32, y: f32, w: f32, h: f32, rgb: u32, alpha: f32) {
    window.paint_quad(fill(
        Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
        color(rgb, alpha),
    ));
}

fn liquid(window: &mut Window, bounds: Bounds<Pixels>, rgb: u32, phase: f32, strength: f32) {
    let x = f32::from(bounds.origin.x);
    let bottom = f32::from(bounds.bottom());
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height).min(160.);
    // Twelve overlapping low-alpha contours feather the wave edge without an
    // offscreen blur texture, backdrop capture, or pane-sized raster allocation.
    for layer in 0..3 {
        for feather in 0..4 {
            let mut path = PathBuilder::fill();
            path.move_to(point(px(x), px(bottom)));
            for sample in 0..=48 {
                let u = sample as f32 / 48.;
                let wave = (u * TAU * 1.4 + phase * TAU + layer as f32 * 1.9).sin()
                    + 0.35 * (u * TAU * 2.8 - phase * TAU).sin();
                let y = bottom - height * (0.24 + layer as f32 * 0.13) + wave * height * 0.065
                    - feather as f32 * 4.;
                path.line_to(point(px(x + u * width), px(y)));
            }
            path.line_to(point(px(x + width), px(bottom)));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color(rgb, (3.5 - feather as f32 * 0.65) * strength));
            }
        }
    }
}

// Small vector bitmap letters avoid per-frame font shaping or glyph-cache work.
// Glyphs: M, O, N, I, T, R, A, X, 0, 1. Each row contains five pixels.
const GLYPHS: [[u8; 7]; 10] = [
    [17, 27, 21, 21, 17, 17, 17],
    [14, 17, 17, 17, 17, 17, 14],
    [17, 25, 25, 21, 19, 19, 17],
    [14, 4, 4, 4, 4, 4, 14],
    [31, 4, 4, 4, 4, 4, 4],
    [30, 17, 17, 30, 20, 18, 17],
    [14, 17, 17, 31, 17, 17, 17],
    [17, 17, 10, 4, 10, 17, 17],
    [14, 17, 19, 21, 25, 17, 14],
    [4, 12, 4, 4, 4, 4, 14],
];

fn matrix(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    rgb: u32,
    phase: f32,
    strength: f32,
    unit: f32,
) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let pixel = (1.25 * unit).max(1.);
    let row_height = pixel * 11.;
    let columns = ((width / (34. * unit)).ceil() as usize).clamp(1, 32);
    for col in 0..columns {
        let seed = ((col * 37 + 11) % 101) as f32 / 101.;
        let head = ((phase + seed).fract() * (height + row_height * 7.)) - row_height;
        for trail in 0..7 {
            let y = head - trail as f32 * row_height;
            if y + pixel * 7. < 0. || y > height {
                continue;
            }
            let edge = (y / height * 3.).clamp(0., 1.);
            let alpha = (1. - trail as f32 / 7.) * 62. * edge * strength;
            let glyph = GLYPHS[(col * 7 + trail * 3 + (phase * 10.) as usize) % GLYPHS.len()];
            let x = left + (col as f32 + 0.5) * width / columns as f32;
            for (row, bits) in glyph.into_iter().enumerate() {
                for bit in 0..5 {
                    if bits & (1 << (4 - bit)) != 0 {
                        rect(
                            window,
                            x + bit as f32 * pixel,
                            top + y + row as f32 * pixel,
                            pixel * 0.8,
                            pixel * 0.8,
                            rgb,
                            alpha,
                        );
                    }
                }
            }
        }
    }
}

fn grid(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    rgb: u32,
    phase: f32,
    strength: f32,
    unit: f32,
) {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let spacing = (30. * unit).max(width / 48.).max(height / 32.);
    let pulse = 0.5 + 0.5 * (phase * TAU).sin();
    for row in 0..=((height / spacing) as usize).min(32) {
        let y = row as f32 * spacing;
        let fade = (y / height).powf(1.5);
        rect(
            window,
            left,
            top + y,
            width,
            0.65 * unit,
            rgb,
            (7. + pulse * 12.) * fade * strength,
        );
    }
    for col in 0..=((width / spacing) as usize).min(48) {
        let x = left + col as f32 * spacing;
        // Segment vertical lines so the entire grid fades softly towards text.
        for segment in 0..8 {
            let y = segment as f32 / 8. * height;
            let fade = ((segment as f32 + 0.5) / 8.).powf(1.5);
            rect(
                window,
                x,
                top + y,
                0.65 * unit,
                height / 8.,
                rgb,
                (7. + pulse * 12.) * fade * strength,
            );
        }
    }
}
