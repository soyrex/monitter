pub use crate::component_traits::{Collapsible, Disableable, Selectable};
pub use crate::sizing::{Sizable, Size, StyleSized};
use gpui::{
    App, BoxShadow, Corners, Div, Edges, FontWeight, Hsla, InteractiveElement as _, ParentElement,
    Pixels, SharedString, StyleRefinement, Styled, Window, div, hsla, prelude::FluentBuilder as _,
    px,
};
pub use gpui_base::{FocusableExt, RoleOverride, StyledExt, box_shadow, h_flex, v_flex};

use crate::ActiveTheme as _;

const FOCUS_RING_WIDTH: Pixels = px(3.);
const FOCUS_RING_OPACITY: f32 = 0.5;

/// Ink every layer of a surface's shadow carries — the `rgb(0 0 0 / 0.1)`
/// shadcn/ui spends at each elevation.
const SURFACE_SHADOW_INK: f32 = 0.1;

/// Shared popup border uses the same theme border as the model picker.
/// Paint its crisp 1px edge through the shadow ring so animated and anchored
/// popups keep identical edges without adding a second outline.
pub(crate) fn popover_ring(cx: &App) -> Hsla {
    cx.theme().border
}

/// Monitter popup elevation: a hairline ring, surrounding ambient shadow,
/// and stronger lower depth shadow. Animation scales their ink together.
/// GPUI blur radii are Gaussian deviations, half the equivalent CSS radius.
/// The ring is supplied separately so animations can retain its theme colour.
pub(crate) fn popover_shadow(ring: Hsla, strength: f32) -> Vec<BoxShadow> {
    let strength = strength.clamp(0., 1.);
    vec![
        // The ring, sitting in the 1px band outside the surface. No blur, so it
        // takes the shader's crisp path rather than the gaussian one.
        BoxShadow::new(px(0.), px(0.), ring.alpha(ring.a * strength))
            .blur_radius(px(0.))
            .spread_radius(px(1.)),
        BoxShadow::new(px(0.), px(0.), hsla(0., 0., 0., 0.18 * strength))
            .blur_radius(px(8.))
            .spread_radius(px(1.)),
        BoxShadow::new(px(0.), px(6.), hsla(0., 0., 0., 0.26 * strength))
            .blur_radius(px(9.))
            .spread_radius(px(0.)),
    ]
}

/// shadcn/ui's `shadow-lg`, the elevation it lifts a toast to, at `strength` of
/// its full ink.
///
/// A toast sits higher than a popover and is built differently: shadcn gives it
/// a real 1px border rather than the translucent ring it puts on a popup, so
/// there is no ring layer here. Its corner radius is left to the caller.
///
/// The radii are Tailwind's halved, for the reason [`popover_shadow`] explains.
pub(crate) fn toast_shadow(strength: f32) -> Vec<BoxShadow> {
    let ink = hsla(0., 0., 0., SURFACE_SHADOW_INK * strength.clamp(0., 1.));
    vec![
        BoxShadow::new(px(0.), px(10.), ink)
            .blur_radius(px(7.5))
            .spread_radius(px(-3.)),
        BoxShadow::new(px(0.), px(4.), ink)
            .blur_radius(px(3.))
            .spread_radius(px(-4.)),
    ]
}

/// shadcn/ui's `shadow-sm`, the elevation it spends on a control raised out of
/// the container it sits in — the active pill of a segmented tab bar — at full
/// ink.
///
/// Unlike a popover or a toast this surface is not floating over the page: it
/// sits *inside* a trough only a few pixels wider than itself, and that trough
/// clips. Both are reasons to keep the falloff tight — there is no room for a
/// wide one, and a wide one would read as grime against the trough wall rather
/// than as lift.
///
/// The radii are Tailwind's halved, for the reason [`popover_shadow`] explains:
/// CSS defines a box shadow's blur radius as twice the gaussian's standard
/// deviation, while GPUI's shader takes the field as the deviation itself
/// (`gaussian(y, sigma)`). Copying Tailwind's `3px` and `2px` across therefore
/// spreads the shadow over twice the distance, which is why
/// [`Styled::shadow_sm`] leaves a haze around a 24px pill where shadcn draws a
/// compact line.
pub(crate) fn raised_shadow() -> Vec<BoxShadow> {
    let ink = hsla(0., 0., 0., SURFACE_SHADOW_INK);
    vec![
        BoxShadow::new(px(0.), px(1.), ink).blur_radius(px(1.5)),
        BoxShadow::new(px(0.), px(1.), ink)
            .blur_radius(px(1.))
            .spread_radius(px(-1.)),
    ]
}

/// Shared title, supporting text and edge-to-edge divider for popup surfaces.
/// Keep this outside the body padding so the divider spans the whole surface.
pub fn popup_header(title: SharedString, subtitle: SharedString, cx: &App) -> Div {
    let title_selector = format!("popup-header-title-{title}");
    let subtitle_selector = format!("popup-header-subtitle-{title}");
    v_flex()
        .when(cfg!(test), |this| {
            this.debug_selector(|| "popup-header-divider".into())
        })
        .min_w_0()
        .flex_none()
        .font_family(cx.theme().font_family.clone())
        .text_size(cx.theme().font_size)
        .text_color(cx.theme().popover_foreground)
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            v_flex()
                .min_w_0()
                .p_3()
                .gap_1()
                .child(div().debug_selector(move || title_selector).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(
                    div()
                        .debug_selector(move || subtitle_selector)
                        .min_w_0()
                        .whitespace_normal()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(subtitle),
                ),
        )
}

/// Finished styles that read the theme.
///
/// Separate from [`StyledExt`], which holds neutral helpers that make no
/// visual decisions. Everything here does: it reaches into the theme and
/// produces a specific look, which is why it belongs above the base layer.
pub trait ThemeStyled: Styled + Sized {
    /// Give this element the focus appearance the framework's own controls
    /// use: its border tinted with the focus colour, and the ring outside it.
    ///
    /// The ring is dropped when [`crate::Theme::focus_ring`] is off, leaving
    /// the tinted border — an application whose layout clips its containers can
    /// turn it off rather than finding room for the ring in each of them.
    ///
    /// Calling this turns the ring on; gate it with `when` for the conditions
    /// that decide whether the control shows one at all — its focus state,
    /// [`FocusableExt::focus_ring`], appearance, and so on.
    ///
    /// The ring sits outside the element's border, so an ancestor that clips
    /// its content will cut it off — leave it a few pixels of room, or don't
    /// clip.
    fn focus_ring_style(self, window: &Window, cx: &App) -> Self
    where
        Self: ParentElement;

    /// Give this element the surface, edge, shadow and radius of a popover.
    ///
    /// This is the one surface every popup shares — Popover, PopupMenu, Select,
    /// Combobox, DatePicker and the editor's hover popovers — so they cannot
    /// drift apart. See [`popover_shadow`] for what the shadow is modelled on.
    fn popover_style(self, cx: &App) -> Self;

    /// Round this element as far as its size allows — a circle if it is square,
    /// a pill if it is not — unless the theme squares its corners.
    ///
    /// Use this instead of [`gpui::Styled::rounded_full`] on anything the theme
    /// owns. A hardcoded `rounded_full` survives [`crate::Theme::radius`] being
    /// set to zero, which leaves avatars, badge dots and slider thumbs round in
    /// a UI that is square everywhere else. See [`crate::Theme::radius_full`].
    fn rounded_full_style(self, cx: &App) -> Self {
        self.rounded(cx.theme().radius_full())
    }
}

impl<T: Styled + Sized> ThemeStyled for T {
    /// Draw the focus ring the framework's own controls use.
    ///
    /// Calling this turns the ring on; gate it with `when` for the conditions
    /// that decide whether the control shows one at all — its focus state,
    /// [`crate::FocusableExt::focus_ring`], appearance, and so on.
    ///
    /// The ring sits outside the element's border, so an ancestor that clips
    /// its content will cut it off — leave it a few pixels of room, or don't
    /// clip.
    fn focus_ring_style(self, window: &Window, cx: &App) -> Self
    where
        Self: ParentElement,
    {
        // The ring is painted outside the border, so a clipping ancestor cuts
        // it off. An application whose layout clips heavily turns it off in the
        // theme and keeps the tinted border, which takes no space.
        if !cx.theme().focus_ring {
            return self.border_color(cx.theme().ring);
        }

        focus_ring(
            self.border_color(cx.theme().ring),
            window,
            cx.theme().ring.alpha(FOCUS_RING_OPACITY),
        )
    }

    fn popover_style(self, cx: &App) -> Self {
        let theme = cx.theme();
        // One visible theme border, painted as a crisp shadow ring, plus the
        // shared ambient and lower drop shadows. Do not add a second border.
        self.bg(gpui::Background::from(theme.popover).backdrop_blur(16.))
            .text_color(theme.popover_foreground)
            .font_family(theme.font_family.clone())
            .text_size(theme.font_size)
            .shadow(popover_shadow(popover_ring(cx), 1.))
            .rounded(theme.radius)
    }
}

/// Paint only the outside band, preserving translucent control backgrounds.
pub(crate) fn focus_ring<T: Styled + ParentElement>(
    mut element: T,
    window: &Window,
    color: Hsla,
) -> T {
    let rem_size = window.rem_size();
    let style = element.style();
    let border_widths = Edges::<Pixels> {
        top: style
            .border_widths
            .top
            .map(|v| v.to_pixels(rem_size))
            .unwrap_or_default(),
        bottom: style
            .border_widths
            .bottom
            .map(|v| v.to_pixels(rem_size))
            .unwrap_or_default(),
        left: style
            .border_widths
            .left
            .map(|v| v.to_pixels(rem_size))
            .unwrap_or_default(),
        right: style
            .border_widths
            .right
            .map(|v| v.to_pixels(rem_size))
            .unwrap_or_default(),
    };
    let radius = Corners::<Pixels> {
        top_left: style
            .corner_radii
            .top_left
            .map(|v| v.to_pixels(rem_size))
            .unwrap_or_default(),
        top_right: style
            .corner_radii
            .top_right
            .map(|v| v.to_pixels(rem_size))
            .unwrap_or_default(),
        bottom_left: style
            .corner_radii
            .bottom_left
            .map(|v| v.to_pixels(rem_size))
            .unwrap_or_default(),
        bottom_right: style
            .corner_radii
            .bottom_right
            .map(|v| v.to_pixels(rem_size))
            .unwrap_or_default(),
    }
    .map(|value| *value + FOCUS_RING_WIDTH);
    let mut ring_style = StyleRefinement::default();
    ring_style.corner_radii.top_left = Some(radius.top_left.into());
    ring_style.corner_radii.top_right = Some(radius.top_right.into());
    ring_style.corner_radii.bottom_left = Some(radius.bottom_left.into());
    ring_style.corner_radii.bottom_right = Some(radius.bottom_right.into());
    let inset = FOCUS_RING_WIDTH;

    element.child(
        div()
            .when(cfg!(test), |this| {
                this.debug_selector(|| "focus-ring".into())
            })
            .flex_none()
            .absolute()
            .top(-(inset + border_widths.top))
            .left(-(inset + border_widths.left))
            .right(-(inset + border_widths.right))
            .bottom(-(inset + border_widths.bottom))
            .border(FOCUS_RING_WIDTH)
            .border_color(color)
            .refine_style(&ring_style),
    )
}
