//! Persistent native companion rail for the existing SubagentPane entity.
//!
//! The owner retains the pane and owns visibility/resizing. This renderer only
//! supplies the rail frame and forwards close to the owner.
use crate::{
    collaboration::SubagentPane,
    design::{self, Palette},
};
use gpui_kit::{
    assets::__private as icon_assets,
    component::{Icon, Sizable, button::*},
    prelude::*,
    *,
};
use serde_json::Value;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailTab {
    Details,
    Timeline,
    Approvals,
    Subagents,
}

impl DetailTab {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Details => 0,
            Self::Timeline => 1,
            Self::Approvals => 2,
            Self::Subagents => 3,
        }
    }

    fn icon(self) -> &'static [u8] {
        match self {
            Self::Details => icon_assets::FileText.1,
            Self::Timeline => icon_assets::Activity.1,
            Self::Approvals => icon_assets::Shield.1,
            Self::Subagents => icon_assets::Bot.1,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Details => "Details",
            Self::Timeline => "Timeline",
            Self::Approvals => "Approvals",
            Self::Subagents => "Subagents",
        }
    }
}

/// Frame a task-owned detail pane. The owner supplies real content and counts;
/// this renderer neither reads the backend nor owns visibility/task selection.
pub fn render_detail_frame<F>(
    pane_id: &str,
    active: DetailTab,
    subagent_count: Option<usize>,
    body: AnyElement,
    palette: Palette,
    settings: &Value,
    focus_handles: &[FocusHandle; 4],
    motion: Rc<std::cell::RefCell<crate::sidebar_tabs_motion::SidebarTabsMotion>>,
    window: &mut Window,
    cx: &mut App,
    on_select: F,
) -> AnyElement
where
    F: Fn(DetailTab, &mut Window, &mut App) + 'static,
{
    let tab_scroll = window.use_keyed_state(format!("{pane_id}-detail-tab-overflow"), cx,
        |_, _| ScrollHandle::new()).read(cx).clone();
    let body_scroll = window.use_keyed_state(format!("{pane_id}-detail-body-overflow"), cx,
        |_, _| ScrollHandle::new()).read(cx).clone();
    let px = design::ui_scaled_px;
    let (header_height, tab_height) = match settings["interfaceDensity"].as_str() {
        Some("tight") => (28_f32, 22_f32),
        Some("spacious") => (38_f32, 30_f32),
        _ => (32_f32, 24_f32),
    };
    let modern = settings["tabStyle"] == "modern";
    let on_select = Rc::new(on_select);
    motion.borrow_mut().observe(active.index());
    let body = motion.borrow().body(pane_id, body, cx.reduce_motion(), false);
    let wheel_select = on_select.clone();
    let mut tabs = div()
        .id(SharedString::from(format!("{pane_id}-detail-tabs")))
        .role(Role::TabList)
        .tab_group()
        .aria_label("Task detail views")
        .flex_none()
        .flex()
        .items_end()
        .gap(px(3.))
        .h(px(header_height))
        .min_w_0()
        .overflow_x_scroll()
        .track_scroll(&tab_scroll)
        .pt(px(6.))
        .px(px(6.))
        .when(modern, |tabs| {
            tabs.h(px((header_height - 2.).max(28.)))
                .p_0()
                .gap_0()
                .items_stretch()
        });
    let tab_order = [
        DetailTab::Details,
        DetailTab::Timeline,
        DetailTab::Approvals,
        DetailTab::Subagents,
    ];
    for (index, tab) in tab_order.into_iter().enumerate() {
        let selected = active == tab;
        let select = on_select.clone();
        let key_select = on_select.clone();
        let click_focus = focus_handles[index].clone();
        let key_focus = (*focus_handles).clone();
        let hover_motion = gpui_kit::component::hover_motion::HoverMotion::new(
            SharedString::from(format!("{pane_id}-detail-{}-hover", tab.label())), true, window, cx,
        );
        let hover_background = hover_motion.color(
            "background", crate::design::surface_color(palette.paper).into(),
            crate::design::surface_color(palette.soft).into(), window, cx,
        );
        let hover_foreground = hover_motion.color(
            "foreground", rgb(if selected { palette.ink } else { palette.muted }).into(),
            rgb(palette.ink).into(), window, cx,
        );
        let mut entry = div()
            .id(SharedString::from(format!(
                "{pane_id}-detail-{}",
                tab.label()
            )))
            .role(Role::Tab)
            .aria_label(tab.label())
            .aria_selected(selected)
            .track_focus(&focus_handles[index])
            .tab_index(index as isize)
            .tab_stop(selected)
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(design::SIDEBAR_TAB_GAP))
            .min_h(px(tab_height))
            .px(px(design::SIDEBAR_TAB_ENTRY_PADDING))
            .rounded_t(px(design::SIDEBAR_TAB_RADIUS))
            .border_1()
            .border_b_0()
            .border_color(rgb(if selected {
                palette.line
            } else {
                palette.paper
            }))
            .when(!selected, |entry| entry.bg(hover_background))
            .text_color(hover_foreground)
            .cursor_pointer()
            .on_hover(move |hovered, _, cx| hover_motion.on_hover(*hovered, cx))
            .when(!selected, |entry| entry.border_b_1())
            .when(modern, |entry| {
                entry
                    .rounded_t(px(0.))
                    .border_0()
                    .border_r_1()
                    .border_color(rgb(palette.line))
            .border_blend_mode(crate::surface_blend::separator_mode())
                    .mb(px(0.))
                    .when(!selected, |entry| entry.border_b_1())
            })
            .focus_visible(move |style| style.border_1().border_color(rgb(palette.accent)))
            .child(
                Icon::default()
                    .data(tab.icon())
                    .text_color(hover_foreground)
                    .with_size(px(design::ICON_SIZE_XS)),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(design::SIDEBAR_TAB_FONT_SIZE))
                    .font_family(design::FONT_MONO)
                    .child(tab.label().to_ascii_uppercase()),
            )
            .on_click(move |_, window, cx| {
                click_focus.focus(window, cx);
                select(tab, window, cx);
            })
            .on_key_down(move |event, window, cx| {
                let modifiers = &event.keystroke.modifiers;
                if modifiers.control || modifiers.platform || modifiers.alt || modifiers.shift {
                    return;
                }
                let next = match event.keystroke.key.as_str() {
                    "left" => (index + 3) % 4,
                    "right" => (index + 1) % 4,
                    "home" => 0,
                    "end" => 3,
                    _ => return,
                };
                key_focus[next].focus(window, cx);
                key_select(tab_order[next], window, cx);
                cx.stop_propagation();
            });
        if tab == DetailTab::Subagents {
            if let Some(count) = subagent_count {
                entry = entry.child(
                    div()
                        .text_size(px(design::SIDEBAR_TAB_FONT_SIZE))
                        .font_family(design::FONT_MONO)
                        .child(count.to_string()),
                );
            }
        }
        tabs = tabs.child(entry);
    }
    div()
        .id(SharedString::from(format!("{pane_id}-detail-frame")))
        .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
            let delta = event.delta.pixel_delta(gpui_kit::px(20.));
            let next = motion.borrow_mut().wheel(delta.x.into(), delta.y.into(), 4, std::time::Instant::now());
            if let Some(index) = next {
                cx.stop_propagation();
                wheel_select(tab_order[index], window, cx);
            }
        })
        .size_full()
        .min_w_0()
        .flex()
        .flex_col()
        .overflow_hidden()
        .border_l_1()
        .border_color(if design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern {
            design::tile_border_color(palette)
        } else { rgb(palette.line) })
        .border_blend_mode(if design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern {
            0
        } else { crate::surface_blend::separator_mode() })
        .bg(crate::surface_blend::secondary_sidebar_background(palette.dark))
        .text_color(rgb(palette.ink))
        .child(ScrollEdgeFade::new(tabs, tab_scroll, Axis::Horizontal, gpui_kit::px(20.)))
        .child(ScrollEdgeFade::new(
            div()
                .id(SharedString::from(format!("{pane_id}-detail-scroll")))
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .track_scroll(&body_scroll)
                .py(px(17.))
                .px(px(14.))
                .child(body),
            body_scroll, Axis::Vertical, gpui_kit::px(20.),
        ))
        .into_any_element()
}

/// Render the Subagents companion rail around an already-retained pane.
///
/// The parent remains responsible for choosing the design-token width,
/// placing this in its resizable layout, retaining the entity, and deciding
/// what close means.
pub fn render_companion<F>(pane: Entity<SubagentPane>, palette: Palette, on_close: F) -> AnyElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let px = design::ui_scaled_px;
    div()
        .id("native-companion-sidebar")
        .w_full()
        .h_full()
        .min_w_0()
        .flex_none()
        .flex()
        .flex_col()
        .overflow_hidden()
        .border_l_1()
        .border_color(if design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern {
            design::tile_border_color(palette)
        } else { rgb(palette.line) })
        .border_blend_mode(if design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern {
            0
        } else { crate::surface_blend::separator_mode() })
        .bg(crate::surface_blend::secondary_sidebar_background(palette.dark))
        .text_color(rgb(palette.ink))
        .child(
            div()
                .id("native-companion-header")
                .h(px(44.))
                .px(px(12.))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(rgb(palette.line))
            .border_blend_mode(crate::surface_blend::separator_mode())
                .child(div().font_weight(FontWeight::MEDIUM).child("Subagents"))
                .child(
                    Button::new("native-companion-close")
                        .compact()
                        .text()
                        .accessibility_label("Close Subagents sidebar")
                        .icon(Icon::default().data(icon_assets::X.1).with_size(px(14.)))
                        .on_click(on_close),
                ),
        )
        .child(
            div()
                .id("native-companion-content")
                .flex_1()
                .min_h_0()
                .child(pane),
        )
        .into_any_element()
}

/// Entity-owned rail clock: unlike keyed element state, it survives cached
/// parent replay and interrupted open/close transitions.
pub(crate) struct RailMotion {
    from: f32,
    target: f32,
    started: std::time::Instant,
    duration: std::time::Duration,
}
impl Default for RailMotion {
    fn default() -> Self {
        Self { from: 0., target: 0., started: std::time::Instant::now(), duration: std::time::Duration::ZERO }
    }
}
impl RailMotion {
    /// Restoring layout or revealing a retained pane installs its final state.
    /// Keep subsequent explicit open/close changes eligible for interpolation.
    pub fn settle(&mut self, present: bool) {
        let target = if present { 1. } else { 0. };
        self.from = target;
        self.target = target;
        self.duration = std::time::Duration::ZERO;
    }

    pub fn running(&self) -> bool {
        !self.duration.is_zero() && self.started.elapsed() < self.duration
    }
    pub fn sample(&mut self, present: bool, reduced: bool) -> (f32, bool) {
        let now = std::time::Instant::now();
        let elapsed = now.saturating_duration_since(self.started);
        let t = if self.duration.is_zero() { 1. } else { (elapsed.as_secs_f32() / self.duration.as_secs_f32()).min(1.) };
        let mut progress = self.from + (self.target - self.from) * (1. - (1. - t).powi(3));
        let target = if present { 1. } else { 0. };
        if reduced {
            self.settle(present);
            return (target, false);
        }
        if self.target != target {
            self.from = progress; self.target = target; self.started = now;
            self.duration = std::time::Duration::from_millis(240).mul_f32((target - progress).abs());
        }
        let running = self.duration > std::time::Duration::ZERO && now.saturating_duration_since(self.started) < self.duration;
        if !running { progress = target; }
        (progress, running)
    }
}

#[cfg(test)]
mod motion_tests {
    use super::RailMotion;

    #[::core::prelude::v1::test]
    fn restored_rail_is_final_immediately_and_manual_toggle_still_animates() {
        let mut motion = RailMotion::default();
        motion.settle(true);
        assert_eq!(motion.sample(true, false), (1., false));
        assert!(!motion.running());
        assert_eq!(motion.sample(false, false), (1., true));
        motion.settle(false);
        assert_eq!(motion.sample(false, false), (0., false));
        assert!(!motion.running());
        assert_eq!(motion.sample(true, false), (0., true));
    }

    #[::core::prelude::v1::test]
    fn restore_cancels_an_in_flight_transition_in_both_directions() {
        let mut motion = RailMotion::default();
        assert!(motion.sample(true, false).1);
        motion.settle(true);
        assert_eq!(motion.sample(true, false), (1., false));
        assert!(motion.sample(false, false).1);
        motion.settle(false);
        assert_eq!(motion.sample(false, false), (0., false));
    }
}
