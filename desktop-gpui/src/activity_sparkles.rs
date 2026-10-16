//! Sparkle overlays for views whose authoritative task is running.
//!
//! Callers own the running-state predicate. A mounted field has one GPUI
//! animation clock and a fixed number of vector stars; it creates no timers,
//! polling, or input handlers of its own.

use gpui_kit::prelude::FluentBuilder as _;
use std::{cell::Cell, rc::Rc};
use std::time::{Duration, Instant};

use crate::design::Palette;
use gpui_kit::{
    Animation, AnimationExt as _, PathBuilder, SharedString, canvas, div, point, px, relative,
    rgba, *,
};

type Stars = Rc<[(f32, f32, f32, f32)]>;

/// One allocation per retained surface, no random work in animation frames.
fn surface_stars(id: &SharedString, layout: SparkleLayout) -> Stars {
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hash);
    (layout as u8).hash(&mut hash);
    let mut state = hash.finish() ^ NEXT.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9e3779b97f4a7c15);
    let mut random = || {
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut mixed = state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d049bb133111eb);
        mixed ^= mixed >> 31;
        ((mixed >> 40) as f32) / 16_777_216.
    };
    let template = match layout {
        SparkleLayout::Compact => &COMPACT_STARS[..],
        SparkleLayout::DockWash | SparkleLayout::HeaderWash => &[][..],
        _ => &PANE_STARS[..],
    };
    template.iter().map(|&(_, _, size, delay)| {
        let x = 3. + random() * 94.;
        let y = if layout == SparkleLayout::Compact { 5. + random() * 90. } else { 15. + random() * 200. };
        (x, y, size, delay)
    }).collect::<Vec<_>>().into()
}

const CYCLE: f32 = 3.4;
const VISIBILITY_FADE: Duration = Duration::from_millis(500);
const PANE_STARS: [(f32, f32, f32, f32); 21] = [
    (6., 40., 11., 0.),
    (92., 90., 9., 0.5),
    (18., 20., 7., 1.4),
    (78., 55., 13., 0.9),
    (40., 130., 8., 2.1),
    (64., 15., 10., 1.7),
    (30., 70., 9., 2.6),
    (88., 160., 7., 3.2),
    (10., 110., 10., 3.6),
    (55., 25., 8., 1.1),
    (72., 185., 11., 2.9),
    (25., 45., 7., 4.1),
    (48., 150., 9., 0.3),
    (82., 205., 8., 1.9),
    (15., 95., 12., 3.9),
    (60., 60., 7., 2.4),
    (95., 30., 9., 0.8),
    (35., 175., 10., 4.5),
    (5., 215., 8., 3.1),
    (68., 120., 7., 1.6),
    (45., 195., 9., 4.8),
];
const COMPACT_STARS: [(f32, f32, f32, f32); 5] = [
    (8., 24., 7., 0.),
    (31., 66., 6., 0.8),
    (56., 18., 8., 1.5),
    (79., 72., 6., 2.2),
    (94., 29., 7., 2.9),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparkleLayout {
    /// Lower half of a pane with stable random-positioned stars.
    Pane,
    /// Full contained field for a task tab.
    Tab,
    /// Shared strip wash, continuing through the adjacent header. No stars.
    DockWash,
    /// Full pane titlebar; descending stars fade beneath the active tab.
    Header,
    /// Static header wash behind title and controls.
    HeaderWash,
    /// Five percentage-positioned stars for a compact sidebar row.
    Compact,
}

/// Shared world-space clock and geometry for a selected tab and its adjacent
/// pane titlebar. The anchor is scoped by the effects registry to window/task.
pub(crate) struct TabAnchor {
    tab_bounds: Cell<Option<Bounds<Pixels>>>,
    dock_bounds: Cell<Option<Bounds<Pixels>>>,
    header_bounds: Cell<Option<Bounds<Pixels>>>,
    epoch: Instant,
}

impl TabAnchor {
    pub(crate) fn new() -> Self {
        Self {
            tab_bounds: Cell::new(None),
            dock_bounds: Cell::new(None),
            header_bounds: Cell::new(None),
            epoch: Instant::now(),
        }
    }
}

impl TabAnchor {
    pub(crate) fn phase_at(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.epoch).as_secs_f32() / CYCLE).rem_euclid(1.)
    }
}

/// Build a clipped, non-interactive sparkle field. Mount only while the task
/// runs. With reduced motion, the same stars remain as a faint static cue.
pub fn build_sparkles(
    id: SharedString,
    layout: SparkleLayout,
    mirror: bool,
    palette: Palette,
    motion_allowed: bool,
) -> AnyElement {
    let stars = surface_stars(&id, layout);
    build_field(SparkleFieldRender {
        active_pane: true,
        stars,
        id,
        layout,
        mirror,
        palette,
        animation: crate::native_motion::ActivityAnimation::Sparkles,
        accent: palette.accent,
        ink: readable_accent(palette),
        dark: palette.dark,
        motion_allowed,
        target_visible: true,
        visibility: None,
        last_painted_phase: Rc::new(Cell::new(None)),
        anchor: None,
        paint_bounds: None,
    })
}

pub(crate) type PaintBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

#[derive(Clone, Default)]
pub(crate) struct ChatDecorationBounds {
    pub header: PaintBounds,
    pub pane: PaintBounds,
    pub composer: PaintBounds,
    pub working_anchor: PaintBounds,
    pub chat_column: PaintBounds,
}

/// Retain one instance per visible running surface. Updating an equal config
/// returns false so callers avoid unnecessary notifications.
pub struct SparkleField {
    obscured: bool,
    active_pane: bool,
    stars: Stars,
    id: SharedString,
    layout: SparkleLayout,
    mirror: bool,
    palette: Palette,
    animation: crate::native_motion::ActivityAnimation,
    ink: u32,
    motion_allowed: bool,
    target_visible: bool,
    visibility: Option<VisibilityFade>,
    visibility_generation: u64,
    last_painted_phase: Rc<Cell<Option<f32>>>,
    anchor: Option<Rc<TabAnchor>>,
    paint_bounds: Option<PaintBounds>,
}

#[derive(Clone, Copy, Debug)]
struct VisibilityFade {
    from: f32,
    to: f32,
    started_at: Instant,
    duration: Duration,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibilityChange {
    pub generation: u64,
    pub target_visible: bool,
    pub cleanup_after: Option<Duration>,
}

impl SparkleField {
    pub(crate) fn set_obscured(&mut self, obscured: bool) -> bool {
        let changed = self.obscured != obscured;
        self.obscured = obscured;
        changed
    }

    pub(crate) fn set_active_pane(&mut self, active: bool) { self.active_pane = active; }

    pub(crate) fn set_paint_bounds(&mut self, bounds: PaintBounds) {
        self.paint_bounds = Some(bounds);
    }

    pub fn new(
        id: SharedString,
        layout: SparkleLayout,
        mirror: bool,
        palette: Palette,
        motion_allowed: bool,
    ) -> Self {
        let stars = surface_stars(&id, layout);
        Self {
            obscured: false,
            active_pane: true,
            stars,
            id,
            layout,
            mirror,
            palette,
            animation: crate::native_motion::ActivityAnimation::Sparkles,
            ink: readable_accent(palette),
            motion_allowed,
            target_visible: false,
            visibility: None,
            visibility_generation: 0,
            last_painted_phase: Rc::new(Cell::new(None)),
            anchor: None,
            paint_bounds: None,
        }
    }

    pub fn set_config(
        &mut self,
        layout: SparkleLayout,
        mirror: bool,
        palette: Palette,
        motion_allowed: bool,
    ) -> bool {
        if self.layout == layout
            && self.mirror == mirror
            && self.palette == palette
            && self.motion_allowed == motion_allowed
        {
            return false;
        }
        if self.layout != layout { self.stars = surface_stars(&self.id, layout); }
        self.layout = layout;
        self.mirror = mirror;
        self.palette = palette;
        self.ink = readable_accent(palette);
        self.motion_allowed = motion_allowed;
        if !motion_allowed && self.visibility.take().is_some() {
            self.visibility_generation = self.visibility_generation.wrapping_add(1);
        }
        true
    }

    pub fn set_animation(&mut self, animation: crate::native_motion::ActivityAnimation) -> bool {
        if self.animation == animation { return false; }
        self.animation = animation;
        true
    }

    pub(crate) fn set_anchor(&mut self, anchor: Option<Rc<TabAnchor>>) {
        self.anchor = anchor;
    }

    pub fn set_visible(&mut self, visible: bool) -> Option<VisibilityChange> {
        self.set_visible_at(visible, Instant::now())
    }

    fn set_visible_at(&mut self, visible: bool, now: Instant) -> Option<VisibilityChange> {
        if self.target_visible == visible {
            return None;
        }
        let current = self.opacity_at(now);
        self.target_visible = visible;
        self.visibility_generation = self.visibility_generation.wrapping_add(1);
        let generation = self.visibility_generation;
        if !self.motion_allowed {
            self.visibility = None;
            return Some(VisibilityChange {
                generation,
                target_visible: visible,
                cleanup_after: (!visible).then_some(Duration::ZERO),
            });
        }

        let target = if visible { 1.0 } else { 0.0 };
        let distance = (target - current).abs();
        if distance <= f32::EPSILON {
            self.visibility = None;
            return Some(VisibilityChange {
                generation,
                target_visible: visible,
                cleanup_after: (!visible).then_some(Duration::ZERO),
            });
        }
        let duration = Duration::from_secs_f32(VISIBILITY_FADE.as_secs_f32() * distance);
        self.visibility = Some(VisibilityFade {
            from: current,
            to: target,
            started_at: now,
            duration,
            generation,
        });
        Some(VisibilityChange {
            generation,
            target_visible: visible,
            cleanup_after: (!visible).then_some(duration),
        })
    }

    fn opacity_at(&self, now: Instant) -> f32 {
        self.visibility
            .map_or(if self.target_visible { 1. } else { 0. }, |fade| {
                fade.opacity_at(now.saturating_duration_since(fade.started_at))
            })
    }

    pub fn visibility_generation(&self) -> u64 {
        self.visibility_generation
    }

    pub fn target_visible(&self) -> bool {
        self.target_visible
    }

    fn visibility_animation(&self, now: Instant) -> Option<VisibilityFade> {
        let fade = self.visibility?;
        (now.saturating_duration_since(fade.started_at) < fade.duration).then_some(fade)
    }
}

impl VisibilityFade {
    fn opacity_at(self, elapsed: Duration) -> f32 {
        if self.duration.is_zero() || elapsed >= self.duration {
            return self.to;
        }
        let progress = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        self.from + (self.to - self.from) * progress.clamp(0., 1.)
    }
}

impl Render for SparkleField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if self.obscured { return div().into_any_element(); }
        build_field(SparkleFieldRender {
            active_pane: self.active_pane,
            stars: self.stars.clone(),
            id: self.id.clone(),
            layout: self.layout,
            mirror: self.mirror,
            palette: self.palette,
            animation: self.animation,
            accent: self.palette.accent,
            ink: self.ink,
            dark: self.palette.dark,
            motion_allowed: self.motion_allowed,
            target_visible: self.target_visible,
            visibility: self.visibility_animation(Instant::now()),
            last_painted_phase: self.last_painted_phase.clone(),
            anchor: self.anchor.clone(),
            paint_bounds: self.paint_bounds.clone(),
        })
    }
}

struct SparkleFieldRender {
    active_pane: bool,
    stars: Stars,
    id: SharedString,
    layout: SparkleLayout,
    mirror: bool,
    palette: Palette,
    animation: crate::native_motion::ActivityAnimation,
    accent: u32,
    ink: u32,
    dark: bool,
    motion_allowed: bool,
    target_visible: bool,
    visibility: Option<VisibilityFade>,
    last_painted_phase: Rc<Cell<Option<f32>>>,
    anchor: Option<Rc<TabAnchor>>,
    paint_bounds: Option<PaintBounds>,
}

fn build_field(config: SparkleFieldRender) -> AnyElement {
    let layout = config.layout;
    let field_id = config.id.clone();
    let phase = retained_phase(
        config.motion_allowed,
        config.target_visible,
        config.last_painted_phase.get(),
    );
    let anchor = config.anchor.clone();
    let field = SparkleCanvas {
        active_pane: config.active_pane,
        stars: config.stars,
        layout,
        mirror: config.mirror,
        palette: config.palette,
        animation: config.animation,
        motion_allowed: config.motion_allowed,
        accent: config.accent,
        ink: config.ink,
        phase,
        anchor: anchor.clone(),
        paint_bounds: config.paint_bounds.clone(),
        follow_anchor: config.motion_allowed && config.target_visible,
    };
    let animated = if config.motion_allowed && config.target_visible && !matches!(layout, SparkleLayout::DockWash | SparkleLayout::HeaderWash) {
        let last_painted_phase = config.last_painted_phase.clone();
        let animation_anchor = anchor.clone();
        let (period, max_fps) = if layout == SparkleLayout::Pane {
            match config.animation {
                crate::native_motion::ActivityAnimation::Liquid => (Duration::from_secs(8), 16.),
                crate::native_motion::ActivityAnimation::Matrix => (Duration::from_millis(4200), 16.),
                crate::native_motion::ActivityAnimation::Grid => (Duration::from_millis(2800), 16.),
                crate::native_motion::ActivityAnimation::Sparkles => (Duration::from_millis(3400), 30.),
            }
        } else {
            (Duration::from_millis(3400), 30.)
        };
        field
            .with_animation(
                SharedString::from(format!("{}-animation-{}", config.id, config.animation.id())),
                Animation::new(period)
                    .repeat_synced()
                    .with_max_fps(max_fps),
                move |mut field, phase| {
                    let current_phase = if matches!(layout, SparkleLayout::Tab | SparkleLayout::Header) {
                        animation_anchor.as_ref().map(|anchor| anchor.phase_at(Instant::now())).unwrap_or(phase)
                    } else {
                        phase
                    };
                    last_painted_phase.set(Some(current_phase));
                    field.phase = Some(current_phase);
                    field
                },
            )
            .into_any_element()
    } else {
        field.into_any_element()
    };
    let field = div()
        .id(field_id)
        .absolute()
        .left(px(0.))
        .right(px(0.))
        .bottom(px(0.))
        .when(layout == SparkleLayout::Pane && config.paint_bounds.is_none(), |field| {
            field.top(relative(0.5))
        })
        .when(layout != SparkleLayout::Pane || config.paint_bounds.is_some(), |field| field.top(px(0.)))
        .overflow_hidden()
        .child(animated);
    let base_opacity = if config.dark {
        1.0
    } else if layout == SparkleLayout::Compact {
        0.65
    } else {
        0.5
    };
    if let Some(visibility) = config.visibility {
        field
            .with_animation(
                SharedString::from(format!(
                    "{}-visibility-{}",
                    config.id, visibility.generation
                )),
                Animation::new(visibility.duration).with_max_fps(30.),
                move |field, progress| {
                    field.opacity(
                        base_opacity
                            * (visibility.from + (visibility.to - visibility.from) * progress),
                    )
                },
            )
            .into_any_element()
    } else {
        field
            .opacity(base_opacity * if config.target_visible { 1. } else { 0. })
            .into_any_element()
    }
}

fn retained_phase(
    motion_allowed: bool,
    target_visible: bool,
    last_painted_phase: Option<f32>,
) -> Option<f32> {
    if !motion_allowed {
        None
    } else if target_visible {
        Some(last_painted_phase.unwrap_or(0.))
    } else {
        last_painted_phase
    }
}

fn continuous_wash_alpha(y: f32, top: f32, bottom: f32) -> f32 {
    if bottom <= top { return 0.; }
    (1. - (y - top) / (bottom - top)).clamp(0., 1.)
}

#[derive(IntoElement)]
struct SparkleCanvas {
    active_pane: bool,
    stars: Stars,
    layout: SparkleLayout,
    mirror: bool,
    palette: Palette,
    animation: crate::native_motion::ActivityAnimation,
    motion_allowed: bool,
    accent: u32,
    ink: u32,
    phase: Option<f32>,
    anchor: Option<Rc<TabAnchor>>,
    paint_bounds: Option<PaintBounds>,
    follow_anchor: bool,
}

impl RenderOnce for SparkleCanvas {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        if self.layout == SparkleLayout::Pane {
            let pattern = match self.animation {
                crate::native_motion::ActivityAnimation::Liquid => Some(crate::activity_patterns::PanePattern::Liquid),
                crate::native_motion::ActivityAnimation::Matrix => Some(crate::activity_patterns::PanePattern::Matrix),
                crate::native_motion::ActivityAnimation::Grid => Some(crate::activity_patterns::PanePattern::Grid),
                crate::native_motion::ActivityAnimation::Sparkles => None,
            };
            if let Some(pattern) = pattern {
                return crate::activity_patterns::build_pattern(
                    SharedString::from("pane-activity-pattern"), pattern, self.palette,
                    self.motion_allowed, self.phase, self.paint_bounds,
                );
            }
        }
        let active_pane = self.active_pane;
        let stars = self.stars;
        let layout = self.layout;
        let mirror = self.mirror;
        let accent = self.accent;
        let ink = self.ink;
        let unit = crate::design::interface_scale() as f32 / 100.;
        let phase = self.phase;
        let anchor = self.anchor.clone();
        let follow_anchor = self.follow_anchor;
        let prepaint_anchor = anchor.clone();
        let paint_bounds = self.paint_bounds.clone();
        let external_bounds = paint_bounds.is_some();
        canvas(
            move |bounds, _, _| {
                let mut bounds = paint_bounds.as_ref().and_then(|value| value.get()).unwrap_or(bounds);
                if external_bounds && layout == SparkleLayout::Pane {
                    bounds.origin.y += bounds.size.height * 0.5;
                    bounds.size.height *= 0.5;
                }
                if layout == SparkleLayout::Tab {
                    if let Some(anchor) = prepaint_anchor.as_ref() {
                        anchor.tab_bounds.set(Some(bounds));
                    }
                }
                if let Some(anchor) = prepaint_anchor.as_ref() {
                    match layout {
                        SparkleLayout::DockWash => anchor.dock_bounds.set(Some(bounds)),
                        SparkleLayout::Header | SparkleLayout::HeaderWash => anchor.header_bounds.set(Some(bounds)),
                        _ => {},
                    }
                }
                bounds
            },
            move |_, bounds, window, _| {
                window.with_content_mask(Some(ContentMask::new(bounds)), |window| {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                if width <= 0. || height <= 0. {
                    return;
                }
                // The outer field owns its light/dark opacity once. Native
                // gradients avoid band edges and per-strip shape allocations.
                let wash = rgba(
                    (accent << 8)
                        | if layout == SparkleLayout::Compact {
                            31
                        } else {
                            51
                        },
                );
                let clear = rgba(accent << 8);
                if layout == SparkleLayout::Pane {
                    let mut pane_wash = wash;
                    if !active_pane { pane_wash.a *= 0.5; }
                    // Inactive working panes retain the same wash at half strength.
                    // The field's lifecycle opacity also handles exit/reduced motion.
                    window.paint_quad(fill(bounds, linear_gradient(
                        180., linear_color_stop(clear, 0.), linear_color_stop(
                            pane_wash, 1.),
                    )));
                }
                if layout == SparkleLayout::Compact {
                    let mut half = bounds;
                    half.size.width *= 0.5;
                    window.paint_quad(fill(
                        half,
                        linear_gradient(
                            90.,
                            linear_color_stop(clear, 0.),
                            linear_color_stop(wash, 1.),
                        ),
                    ));
                    half.origin.x += half.size.width;
                    window.paint_quad(fill(
                        half,
                        linear_gradient(
                            90.,
                            linear_color_stop(wash, 0.),
                            linear_color_stop(clear, 1.),
                        ),
                    ));
                } else if matches!(layout, SparkleLayout::DockWash | SparkleLayout::HeaderWash) || (layout == SparkleLayout::Header && !external_bounds) {
                    // Both regions sample one world-space ramp. No per-tab glow,
                    // pane wash, or fresh alpha peak at the header seam.
                    let dock = anchor.as_ref().and_then(|a| a.dock_bounds.get());
                    let header = anchor.as_ref().and_then(|a| a.header_bounds.get());
                    let top = dock.map_or(f32::from(bounds.origin.y), |b| f32::from(b.origin.y));
                    let bottom = header.map_or(f32::from(bounds.bottom()), |b| f32::from(b.bottom()));
                    let alpha_at = |y| continuous_wash_alpha(y, top, bottom);
                    let color_at = |y| rgba((accent << 8) | (51. * alpha_at(y)).round() as u32);
                    // The ramp still spans dock+header vertically, but its strip
                    // segment belongs only to the selected task's actual tab.
                    // Empty strip space retains its ordinary paper surface.
                    let wash_bounds = if layout == SparkleLayout::DockWash {
                        let Some(tab) = anchor.as_ref().and_then(|a| a.tab_bounds.get()) else { return; };
                        bounds.intersect(&tab)
                    } else {
                        bounds
                    };
                    window.paint_quad(fill(wash_bounds, linear_gradient(
                        180.,
                        linear_color_stop(color_at(f32::from(wash_bounds.origin.y)), 0.),
                        linear_color_stop(color_at(f32::from(wash_bounds.bottom())), 1.),
                    )));
                }
                if matches!(layout, SparkleLayout::DockWash | SparkleLayout::HeaderWash) { return; }


                let shared_geometry = if layout == SparkleLayout::Header {
                    anchor.as_ref().and_then(|anchor| {
                        anchor
                            .tab_bounds
                            .get()
                            .filter(|tab| adjacent_tab_bounds(*tab, bounds))
                    })
                } else {
                    None
                };
                let effective_phase = if follow_anchor && matches!(layout, SparkleLayout::Tab | SparkleLayout::Header) {
                    anchor.as_ref().map(|anchor| anchor.phase_at(Instant::now())).or(phase)
                } else {
                    phase
                };
                for &(x, source_y, source_size, delay) in stars.iter() {
                    let local = effective_phase.map(|time| (time - delay / CYCLE).rem_euclid(1.0));
                    let (opacity, rise, scale) = local.map_or((0.55, 0., 1.), sparkle_frame);
                    if opacity <= 0. {
                        continue;
                    }
                    let star_size = source_size * scale * unit;
                    let position_bounds = shared_geometry.unwrap_or(bounds);
                    let position_width = f32::from(position_bounds.size.width);
                    let mut center_x = f32::from(position_bounds.origin.x) + position_width * x / 100.;
                    let mut center_y = if layout == SparkleLayout::Compact {
                        f32::from(bounds.origin.y) + height * (1. - source_y / 100.)
                    } else {
                        star_center_y(position_bounds, source_y, unit, mirror)
                    };
                    // CSS translateY(+14px .. -18px), reversed by the tab's
                    // vertical mirror.
                    if local.is_some() {
                        center_y += if mirror { rise * unit } else { -rise * unit };
                    }
                    // Use the source's accent-derived ink color, retaining the
                    // selected accent hue instead of substituting white/black.
                    // The titlebar is the fading continuation below the tab;
                    // its stars disappear toward the conversation edge too.
                    let header_fade = if layout == SparkleLayout::Header {
                        (1. - (center_y - f32::from(bounds.origin.y)) / height).clamp(0., 1.)
                    } else {
                        1.
                    };
                    let star_alpha = (opacity * header_fade * 255.) as u8;
                    draw_star(
                        window,
                        center_x,
                        center_y,
                        star_size * 1.8,
                        accent,
                        star_alpha / 4,
                    );
                    draw_star(window, center_x, center_y, star_size, ink, star_alpha);
                }
                });
            },
        )
        .size_full()
        .into_any_element()
    }
}

fn adjacent_tab_bounds(tab: Bounds<Pixels>, header: Bounds<Pixels>) -> bool {
    let tab_bottom = f32::from(tab.origin.y + tab.size.height);
    let header_top = f32::from(header.origin.y);
    let horizontal_overlap = f32::from(tab.origin.x)
        < f32::from(header.origin.x + header.size.width)
        && f32::from(header.origin.x) < f32::from(tab.origin.x + tab.size.width);
    (tab_bottom - header_top).abs() <= 2. && horizontal_overlap
}

fn star_center_y(bounds: Bounds<Pixels>, source_y: f32, unit: f32, mirror: bool) -> f32 {
    let origin = f32::from(bounds.origin.y);
    let height = f32::from(bounds.size.height);
    let y = origin + height - source_y * unit;
    if mirror {
        origin + height - (y - origin)
    } else {
        y
    }
}

#[cfg(test)]
mod anchor_tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn only_adjacent_overlapping_header_continues_the_tab_world_field() {
        let tab = Bounds::new(point(px(100.), px(200.)), size(px(300.), px(32.)));
        let adjacent = Bounds::new(point(px(80.), px(232.)), size(px(500.), px(40.)));
        let foreign_pane = Bounds::new(point(px(420.), px(232.)), size(px(200.), px(40.)));
        let gapped = Bounds::new(point(px(80.), px(236.)), size(px(500.), px(40.)));

        assert!(adjacent_tab_bounds(tab, adjacent));
        assert!(!adjacent_tab_bounds(tab, foreign_pane));
        assert!(!adjacent_tab_bounds(tab, gapped));
    }

    #[::core::prelude::v1::test]
    fn mirrored_tab_and_header_stars_keep_the_same_world_y() {
        let tab = Bounds::new(point(px(0.), px(100.)), size(px(280.), px(32.)));
        let header = Bounds::new(point(px(0.), px(132.)), size(px(560.), px(48.)));
        let y_in_tab = star_center_y(tab, 45., 1., true);
        let y_in_header = star_center_y(tab, 45., 1., true);

        assert!(adjacent_tab_bounds(tab, header));
        assert_eq!(y_in_tab, 145.);
        assert_eq!(y_in_tab, y_in_header);
    }
}

// Same 4.5:1 readable accent projection used by Tauri. Cache per palette.
fn readable_accent(palette: Palette) -> u32 {
    fn channels(rgb: u32) -> [f32; 3] {
        [
            (rgb >> 16 & 255) as f32,
            (rgb >> 8 & 255) as f32,
            (rgb & 255) as f32,
        ]
    }
    fn luminance(ch: [f32; 3]) -> f32 {
        ch.into_iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(c, weight)| {
                let v = c / 255.;
                weight
                    * if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum()
    }
    let color = channels(palette.accent);
    let background = luminance(channels(palette.paper));
    let target = if palette.dark { 255. } else { 0. };
    for step in 0..=100 {
        let ch = color.map(|c| (c + (target - c) * step as f32 / 100.).round());
        let lum = luminance(ch);
        if (lum.max(background) + 0.05) / (lum.min(background) + 0.05) >= 4.5 {
            return (ch[0] as u32) << 16 | (ch[1] as u32) << 8 | ch[2] as u32;
        }
    }
    if palette.dark { 0xffffff } else { 0 }
}

fn sparkle_frame(phase: f32) -> (f32, f32, f32) {
    if phase < 0.15 {
        let t = ease_in(phase / 0.15);
        (t, -14. * (1. - t), 0.5 + 0.5 * t)
    } else if phase < 0.45 {
        let t = ease_in((phase - 0.15) / 0.30);
        (1. - 0.15 * t, 4. * t, 1.)
    } else if phase < 0.70 {
        let t = ease_in((phase - 0.45) / 0.25);
        (0.85 * (1. - t), 4. + 14. * t, 1. - 0.4 * t)
    } else {
        (0., 18., 0.6)
    }
}

// CSS ease-in applies to each interval between keyframes, not the whole cycle.
fn ease_in(x: f32) -> f32 {
    if x <= 0. {
        return 0.;
    }
    if x >= 1. {
        return 1.;
    }
    let (mut low, mut high) = (0., 1.);
    for _ in 0..14 {
        let t = (low + high) * 0.5;
        let u = 1. - t;
        let bezier_x = 3. * u * u * t * 0.42 + 3. * u * t * t + t * t * t;
        if bezier_x < x {
            low = t;
        } else {
            high = t;
        }
    }
    let t = (low + high) * 0.5;
    3. * (1. - t) * t * t + t * t * t
}

fn draw_star(window: &mut Window, center_x: f32, center_y: f32, size: f32, color: u32, alpha: u8) {
    // Reject paths before trigonometry, tessellation and scaled vertex allocation.
    // Keep the full radius: a star or its larger halo can cross a clip edge even
    // when its center is outside. Scene clipping still handles partial stars.
    if !star_is_visible(
        center_x,
        center_y,
        size,
        alpha,
        window.content_mask().bounds,
    ) {
        return;
    }
    if let Some(path) = star_path(center_x, center_y, size) {
        window.paint_path(path, rgba((color << 8) | u32::from(alpha)));
    }
}

thread_local! {
    // The contour only changes by positive uniform scale and translation.
    // Keep one tessellation per drawing thread instead of rebuilding Lyon's
    // path, tessellator and vertex/index buffers for every ink and halo.
    static STAR_MESH: Option<Path<Pixels>> = build_star_mesh();
}

fn build_star_mesh() -> Option<Path<Pixels>> {
    let mut path = PathBuilder::fill();
    for point_index in 0..8 {
        let angle = -std::f32::consts::FRAC_PI_2 + point_index as f32 * std::f32::consts::FRAC_PI_4;
        let point_radius = if point_index % 2 == 0 { 1. } else { 0.22 };
        let p = point(
            px(angle.cos() * point_radius),
            px(angle.sin() * point_radius),
        );
        if point_index == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close();
    path.build().ok()
}

fn star_path(center_x: f32, center_y: f32, size: f32) -> Option<Path<Pixels>> {
    STAR_MESH.with(|mesh| {
        let mut path = mesh.as_ref()?.clone();
        let radius = size * 0.5;
        let transform = |p: Point<Pixels>| {
            point(
                px(center_x + f32::from(p.x) * radius),
                px(center_y + f32::from(p.y) * radius),
            )
        };
        // Like plot::PathCache's translation, only finished bounds and vertices
        // are consumed by paint_path. The builder cursor is never used again.
        path.bounds = Bounds::from_corners(
            transform(path.bounds.origin),
            transform(path.bounds.bottom_right()),
        );
        for vertex in &mut path.vertices {
            vertex.xy_position = transform(vertex.xy_position);
        }
        Some(path)
    })
}

fn star_is_visible(
    center_x: f32,
    center_y: f32,
    size: f32,
    alpha: u8,
    clip: Bounds<Pixels>,
) -> bool {
    if alpha == 0 || clip.is_empty() {
        return false;
    }
    // A small conservative fringe also avoids inward float rounding when a
    // path's bounds reconstruct the far edge from origin + width.
    let radius = size * 0.5 + 1.;
    center_x + radius > f32::from(clip.left())
        && center_x - radius < f32::from(clip.right())
        && center_y + radius > f32::from(clip.top())
        && center_y - radius < f32::from(clip.bottom())
}

#[cfg(test)]
mod tests {
    use super::{
        COMPACT_STARS, CYCLE, PANE_STARS, SparkleField, VisibilityFade, retained_phase,
        sparkle_frame, star_is_visible, star_path,
    };
    use std::time::{Duration, Instant};

    fn reference_star_points(x: f32, y: f32, size: f32) -> [gpui_kit::Point<gpui_kit::Pixels>; 8] {
        let radius = size * 0.5;
        let inner = radius * 0.22;
        std::array::from_fn(|i| {
            let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::FRAC_PI_4;
            let radius = if i % 2 == 0 { radius } else { inner };
            gpui_kit::point(
                gpui_kit::px(x + angle.cos() * radius),
                gpui_kit::px(y + angle.sin() * radius),
            )
        })
    }

    fn reference_star_path(x: f32, y: f32, size: f32) -> gpui_kit::Path<gpui_kit::Pixels> {
        let mut path = gpui_kit::PathBuilder::fill();
        for (i, point) in reference_star_points(x, y, size).into_iter().enumerate() {
            if i == 0 {
                path.move_to(point);
            } else {
                path.line_to(point);
            }
        }
        path.close();
        path.build().unwrap()
    }

    fn xy(p: gpui_kit::Point<gpui_kit::Pixels>) -> [f64; 2] {
        [f64::from(f32::from(p.x)), f64::from(f32::from(p.y))]
    }

    fn cross(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    }

    fn mesh_area(path: &gpui_kit::Path<gpui_kit::Pixels>) -> f64 {
        path.vertices
            .chunks_exact(3)
            .map(|t| {
                cross(
                    xy(t[0].xy_position),
                    xy(t[1].xy_position),
                    xy(t[2].xy_position),
                )
                .abs()
                    * 0.5
            })
            .sum()
    }

    fn mesh_covers(path: &gpui_kit::Path<gpui_kit::Pixels>, p: [f64; 2]) -> bool {
        path.vertices.chunks_exact(3).any(|t| {
            let [a, b, c] = [
                xy(t[0].xy_position),
                xy(t[1].xy_position),
                xy(t[2].xy_position),
            ];
            if cross(a, b, c).abs() <= f64::EPSILON {
                return false;
            }
            let sides = [cross(a, b, p), cross(b, c, p), cross(c, a, p)];
            sides.iter().all(|side| *side >= 0.) || sides.iter().all(|side| *side <= 0.)
        })
    }

    fn distance_to_contour(points: &[gpui_kit::Point<gpui_kit::Pixels>; 8], p: [f64; 2]) -> f64 {
        (0..8)
            .map(|i| {
                let a = xy(points[i]);
                let b = xy(points[(i + 1) % 8]);
                let d = [b[0] - a[0], b[1] - a[1]];
                let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1])
                    / (d[0] * d[0] + d[1] * d[1]))
                    .clamp(0., 1.);
                (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
            })
            .fold(f64::INFINITY, f64::min)
    }

    #[::core::prelude::v1::test]
    fn cached_star_mesh_matches_fresh_tessellation_over_motion_and_geometry() {
        let mut checked_samples = 0;
        for ui_scale in [0.75, 1., 1.5, 2.] {
            for phase in [0.01, 0.14, 0.15, 0.4, 0.69] {
                let (_, _, scale) = sparkle_frame(phase);
                for source_size in [6., 9., 13.] {
                    for halo in [1., 1.8] {
                        let size = source_size * scale * ui_scale * halo;
                        for (x, y) in [
                            (0., 0.),
                            (13.25, 29.5),
                            (-33.5, 92.125),
                            (3818.873, 2048.25),
                        ] {
                            let cached = star_path(x, y, size).unwrap();
                            let reference = reference_star_path(x, y, size);
                            // Reassociation of scale/translation can move an
                            // edge by a few f32 ULPs, especially far from zero.
                            let tolerance = 0.00001
                                + f64::from(x.abs().max(y.abs()).max(size))
                                    * f64::from(f32::EPSILON)
                                    * 4.;
                            for (a, b) in [
                                (cached.bounds.left(), reference.bounds.left()),
                                (cached.bounds.right(), reference.bounds.right()),
                                (cached.bounds.top(), reference.bounds.top()),
                                (cached.bounds.bottom(), reference.bounds.bottom()),
                            ] {
                                assert!(
                                    (f64::from(f32::from(a)) - f64::from(f32::from(b))).abs()
                                        <= tolerance
                                );
                            }
                            assert!(
                                (mesh_area(&cached) - mesh_area(&reference)).abs()
                                    <= tolerance * f64::from(size) * 4.
                            );
                            let contour = reference_star_points(x, y, size);
                            for ix in 0..15 {
                                for iy in 0..15 {
                                    let p = [
                                        f64::from(x)
                                            + (-1.05 + ix as f64 * 0.15) * f64::from(size) * 0.5,
                                        f64::from(y)
                                            + (-1.025 + iy as f64 * 0.15) * f64::from(size) * 0.5,
                                    ];
                                    if distance_to_contour(&contour, p) > tolerance {
                                        assert_eq!(
                                            mesh_covers(&cached, p),
                                            mesh_covers(&reference, p),
                                            "geometry x={x} y={y} size={size} sample={p:?}"
                                        );
                                        checked_samples += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(checked_samples > 100_000);
    }

    #[::core::prelude::v1::test]
    fn transformed_mesh_does_not_mutate_the_cached_template() {
        let baseline = star_path(0., 0., 2.).unwrap();
        let mut moved = star_path(75.5, -38.25, 17.).unwrap();
        moved.vertices[0].xy_position = gpui_kit::point(gpui_kit::px(999.), gpui_kit::px(999.));
        let again = star_path(0., 0., 2.).unwrap();
        assert_eq!(baseline.bounds, again.bounds);
        assert_eq!(baseline.vertices.len(), again.vertices.len());
        for (a, b) in baseline.vertices.iter().zip(&again.vertices) {
            assert_eq!(a.xy_position, b.xy_position);
            assert_eq!(a.st_position, b.st_position);
        }
    }

    #[::core::prelude::v1::test]
    fn transformed_mesh_keeps_the_far_vertex_at_a_neighboring_float_clip_edge() {
        use gpui_kit::{Bounds, point, px, size};
        let path = star_path(3818.873, 50., 10.417278).unwrap();
        let far_vertex = path
            .vertices
            .iter()
            .map(|v| f32::from(v.xy_position.x))
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(f32::from(path.bounds.right()) >= far_vertex);
        let clip = Bounds::new(point(px(3824.0815), px(0.)), size(px(10.), px(100.)));
        assert!(far_vertex > f32::from(clip.left()));
        assert!(!path.bounds.intersect(&clip).is_empty());
    }

    #[::core::prelude::v1::test]
    fn clipping_keeps_partial_stars_and_independent_halos() {
        use gpui_kit::{Bounds, point, px, size};
        let clip = Bounds::new(point(px(10.), px(20.)), size(px(100.), px(32.)));
        assert!(!star_is_visible(60., 36., 10., 0, clip));
        assert!(star_is_visible(60., 36., 10., 1, clip));
        for (x, y) in [(8., 36.), (112., 36.), (60., 18.), (60., 54.)] {
            assert!(star_is_visible(x, y, 10., 255, clip));
        }
        for (x, y) in [(4., 36.), (116., 36.), (60., 14.), (60., 58.)] {
            assert!(!star_is_visible(x, y, 10., 255, clip));
            assert!(star_is_visible(x, y, 18., 63, clip));
        }
        assert!(!star_is_visible(60., 36., 10., 255, Bounds::default()));

        // Reconstructing the star's right edge as (center - radius) + size
        // rounds inward at these coordinates; retain the visible sliver.
        let clip = Bounds::new(point(px(3824.0815), px(0.)), size(px(10.), px(100.)));
        assert!(star_is_visible(3818.873, 50., 10.417278, 255, clip));
    }

    #[::core::prelude::v1::test]
    fn early_clipping_preserves_every_path_the_scene_could_draw() {
        use gpui_kit::{Bounds, PathBuilder, point, px, size};
        // Compare against the actual tessellated path over phases, UI scales,
        // clip edges and both ink/halo sizes, rather than testing centers only.
        let clip = Bounds::new(point(px(13.25), px(29.5)), size(px(80.5), px(32.)));
        let mut culled = 0;
        let mut visible = 0;
        for ui_scale in [0.75, 1., 1.5, 2.] {
            for phase in [0.01, 0.14, 0.15, 0.4, 0.69] {
                let (opacity, _, scale) = sparkle_frame(phase);
                for halo in [1., 1.8] {
                    let star_size = 13. * scale * ui_scale * halo;
                    let radius = star_size * 0.5;
                    let alpha = (opacity * 255.) as u8;
                    for x in [0., 10., 13.25, 50., 93.75, 98., 120.] {
                        for y in [0., 24., 29.5, 45., 61.5, 66., 90.] {
                            let mut path = PathBuilder::fill();
                            for i in 0..8 {
                                let angle = -std::f32::consts::FRAC_PI_2
                                    + i as f32 * std::f32::consts::FRAC_PI_4;
                                let r = if i % 2 == 0 { radius } else { radius * 0.22 };
                                let p = point(px(x + angle.cos() * r), px(y + angle.sin() * r));
                                if i == 0 { path.move_to(p); } else { path.line_to(p); }
                            }
                            path.close();
                            let path = path.build().unwrap();
                            if !star_is_visible(x, y, star_size, alpha, clip) {
                                culled += 1;
                                assert!(alpha == 0 || path.bounds.intersect(&clip).is_empty());
                            } else {
                                visible += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(culled > 0 && visible > 0, "exercise both sides of the clipping decision");
    }

    #[::core::prelude::v1::test]
    fn source_geometry_and_stagger_are_bounded() {
        assert_eq!(PANE_STARS.len(), 21);
        assert_eq!(COMPACT_STARS.len(), 5);
        assert!(PANE_STARS.iter().all(|(x, y, size, delay)| {
            (0. ..=100.).contains(x)
                && (0. ..=215.).contains(y)
                && (7. ..=13.).contains(size)
                && (0. ..=CYCLE * 1.5).contains(delay)
        }));
        assert_eq!(PANE_STARS[20].3, 4.8);
    }

    #[::core::prelude::v1::test]
    fn star_cycle_has_source_fade_and_static_endpoint_values() {
        assert_eq!(sparkle_frame(0.), (0., -14., 0.5));
        assert_eq!(sparkle_frame(0.15), (1., 0., 1.));
        assert_eq!(sparkle_frame(0.70), (0., 18., 0.6));
    }

    #[::core::prelude::v1::test]
    fn exit_fade_keeps_the_last_animated_phase_without_advancing_it() {
        let last_painted = Some(0.731);
        assert_eq!(retained_phase(true, false, last_painted), last_painted);
        assert_eq!(retained_phase(true, true, last_painted), last_painted);
        assert_eq!(retained_phase(true, true, None), Some(0.));
        assert_eq!(retained_phase(true, false, None), None);
        assert_eq!(retained_phase(false, false, last_painted), None);
    }

    #[::core::prelude::v1::test]
    fn visibility_fades_in_and_out_over_500ms_and_reverses_without_a_jump() {
        let palette =
            crate::design::palette(&serde_json::json!({}), gpui_kit::WindowAppearance::Dark);
        let start = Instant::now();
        let mut field = SparkleField::new(
            gpui_kit::SharedString::from("test-sparkle"),
            super::SparkleLayout::Pane,
            false,
            palette,
            true,
        );
        let entrance = field.set_visible_at(true, start).unwrap();
        assert_eq!(entrance.cleanup_after, None);
        let enter = VisibilityFade {
            from: 0.,
            to: 1.,
            started_at: start,
            duration: Duration::from_millis(500),
            generation: entrance.generation,
        };
        assert_eq!(enter.opacity_at(Duration::ZERO), 0.);
        assert_eq!(enter.opacity_at(Duration::from_millis(250)), 0.5);
        assert_eq!(enter.opacity_at(Duration::from_millis(500)), 1.);

        // Reverse halfway through the entry: sample the current alpha, then
        // travel only the remaining distance to zero.
        field.visibility = Some(enter);
        let reversal_time = start + Duration::from_millis(250);
        let reversal = field.set_visible_at(false, reversal_time).unwrap();
        let exit = field.visibility.unwrap();
        assert_eq!(exit.from, 0.5);
        assert_eq!(exit.to, 0.);
        assert_eq!(exit.duration, Duration::from_millis(250));
        assert_eq!(exit.opacity_at(Duration::ZERO), 0.5);
        assert_eq!(exit.opacity_at(exit.duration), 0.);
        assert_ne!(reversal.generation, entrance.generation);
        assert_eq!(reversal.cleanup_after, Some(Duration::from_millis(250)));
    }

    #[::core::prelude::v1::test]
    fn reduced_motion_resolves_visibility_without_a_fade() {
        let palette =
            crate::design::palette(&serde_json::json!({}), gpui_kit::WindowAppearance::Dark);
        let mut field = SparkleField::new(
            gpui_kit::SharedString::from("static-sparkle"),
            super::SparkleLayout::Pane,
            false,
            palette,
            false,
        );
        let show = field.set_visible_at(true, Instant::now()).unwrap();
        assert_eq!(show.cleanup_after, None);
        assert!(field.target_visible());
        assert!(field.visibility.is_none());
        let hide = field.set_visible_at(false, Instant::now()).unwrap();
        assert_eq!(hide.cleanup_after, Some(Duration::ZERO));
        assert!(!field.target_visible());
        assert!(field.visibility.is_none());
    }
}

#[cfg(test)]
mod continuous_wash_tests {
    #[test]
    fn tab_and_header_share_one_monotonic_ramp() {
        let alpha = |y| super::continuous_wash_alpha(y, 20., 140.);
        assert_eq!(alpha(20.), 1.);
        assert_eq!(alpha(140.), 0.);
        let seam = alpha(60.);
        assert!((seam - 2. / 3.).abs() < 0.00001);
        for y in 20..140 { assert!(alpha(y as f32) >= alpha(y as f32 + 1.)); }
    }
}
