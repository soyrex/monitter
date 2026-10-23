//! Animated run controls live beside cached ChatPane, never beneath it.
use crate::{
    activity_sparkles::PaintBounds,
    design::{self, ui_scaled_px as px},
};
use gpui_kit::{
    assets::IconName,
    component::{Disableable, Icon, Sizable, button::*, popover::Popover, spinner::Spinner},
    prelude::*,
    *,
};
use serde_json::Value;

fn task_progress(snapshot: &Value, task_id: &str) -> Option<String> {
    let user_at = snapshot
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| message["taskId"] == task_id && message["role"] == "user")
        .filter_map(|message| message["createdAt"].as_i64())
        .max()
        .unwrap_or_default();
    snapshot
        .get("events")?
        .as_array()?
        .iter()
        .filter(|event| {
            event["taskId"] == task_id
                && event["kind"] == "agent_progress"
                && event["createdAt"].as_i64().unwrap_or_default() >= user_at
        })
        .max_by_key(|event| event["createdAt"].as_i64().unwrap_or_default())?["title"]
        .as_str()
        .map(str::to_owned)
}

fn active_work_plan<'a>(snapshot: &'a Value, task_id: &str) -> Option<&'a Value> {
    snapshot
        .get("workPlans")?
        .as_array()?
        .iter()
        .filter(|plan| plan["taskId"].as_str() == Some(task_id) && plan["status"] == "active")
        .max_by_key(|plan| {
            (
                plan["updatedAt"].as_i64().unwrap_or_default(),
                plan["createdAt"].as_i64().unwrap_or_default(),
            )
        })
}

fn plan_popup(
    plan: &Value,
    mut palette: design::Palette,
    width: Pixels,
    height: Pixels,
    font_ratio: f32,
) -> AnyElement {
    // The popup always uses a dark glass surface, independent of pane theme.
    let dark = design::palette(&Value::Null, WindowAppearance::Dark);
    palette.dark = true;
    palette.ink = dark.ink;
    palette.muted = dark.muted;
    palette.line = dark.line;
    palette.panel = 0x000000;
    let background: Background = rgb(0x000000).opacity(0.8).into();
    div()
        .id("working-plan-popup")
        .debug_selector(|| "working-plan-popup".into())
        .w(width)
        .max_h(height)
        .min_h_0()
        .overflow_y_scroll()
        .rounded(px(design::radius::MODAL))
        .bg(background.backdrop_blur(20.))
        .child(crate::plans::render_plan_card(
            plan, palette, font_ratio, true,
        ))
        .into_any_element()
}

pub(crate) enum WorkingBubbleEvent {
    Stop,
}
pub(crate) struct WorkingBubble {
    composer: PaintBounds,
    chat_column: PaintBounds,
    palette: design::Palette,
    visible: bool,
    starting: bool,
    stop_requested: bool,
    active_plan: Option<Value>,
    header_plan_access: bool,
    progress: Option<String>,
    plan_open: bool,
    font_ratio: f32,
}
impl EventEmitter<WorkingBubbleEvent> for WorkingBubble {}
impl WorkingBubble {
    pub(crate) fn new(
        composer: PaintBounds,
        chat_column: PaintBounds,
        palette: design::Palette,
    ) -> Self {
        Self {
            composer,
            chat_column,
            palette,
            visible: false,
            starting: false,
            stop_requested: false,
            active_plan: None,
            header_plan_access: false,
            progress: None,
            plan_open: false,
            font_ratio: 1.,
        }
    }
    /// Chat panes expose the plan in their header popup instead of the run pill.
    pub(crate) fn with_header_plan_access(mut self) -> Self {
        self.header_plan_access = true;
        self
    }
    pub(crate) fn set_context(
        &mut self,
        visible: bool,
        starting: bool,
        stop_requested: bool,
        palette: design::Palette,
        snapshot: &Value,
        task_id: &str,
        cx: &mut Context<Self>,
    ) {
        let next_progress = task_progress(snapshot, task_id);
        let progress_changed = self.progress != next_progress;
        self.progress = next_progress;
        let next_plan = active_work_plan(snapshot, task_id);
        let plan_changed = self.active_plan.as_ref() != next_plan;
        let next_ratio = snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        if plan_changed {
            let old_id = self
                .active_plan
                .as_ref()
                .and_then(|plan| plan["id"].as_str());
            let new_id = next_plan.and_then(|plan| plan["id"].as_str());
            if old_id != new_id {
                self.plan_open = false;
            }
            self.active_plan = next_plan.cloned();
        }
        if !visible || self.active_plan.is_none() {
            self.plan_open = false;
        }
        if (
            self.visible,
            self.starting,
            self.stop_requested,
            self.palette,
        ) != (visible, starting, stop_requested, palette)
            || plan_changed
            || progress_changed
            || self.font_ratio != next_ratio
        {
            self.visible = visible;
            self.starting = starting;
            self.stop_requested = stop_requested;
            self.palette = palette;
            self.font_ratio = next_ratio;
            cx.notify();
        }
    }
}
impl Render for WorkingBubble {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().into_any_element();
        }
        let p = self.palette;
        let column = self.chat_column.get();
        // Keep the action hitboxes available even when a narrow pane or a
        // larger text preference leaves little room for the chrome labels.
        let compact_controls =
            column.is_some_and(|column| column.size.width < px(280. * self.font_ratio.max(1.)));
        let status = if self.stop_requested {
            "Stop queued…"
        } else if self.starting {
            "Starting…"
        } else {
            self.progress.as_deref().unwrap_or("Working…")
        }
        .to_owned();
        let status_preview = if status.chars().count() > 32 {
            format!("{}…", status.chars().take(31).collect::<String>())
        } else {
            status.clone()
        };
        let mut pill = div()
            .id("working-bubble")
            .debug_selector(|| "working-bubble".into())
            .flex()
            .when_some(column, |pill, column| pill.max_w(column.size.width))
            .items_center()
            .gap(px(0.))
            .rounded_full()
            .border_1()
            .border_color(rgb(p.hairline(1)))
            .bg(crate::surface_blend::floating_control_background(
                p.panel, p.dark,
            ))
            .text_color(rgb(p.text(1)))
            .text_size(px(12. * self.font_ratio))
            .line_height(px(16. * self.font_ratio));
        if let Some(plan) = self
            .active_plan
            .clone()
            .filter(|_| !self.header_plan_access)
        {
            let owner = cx.weak_entity();
            let width = self
                .chat_column
                .get()
                .map(|column| (column.size.width - px(24.)).max(px(1.)).min(px(400.)))
                .unwrap_or(px(400.));
            let font_ratio = self.font_ratio;
            let anchor_bounds =
                self.chat_column
                    .get()
                    .zip(self.composer.get())
                    .map(|(column, composer)| {
                        Bounds::new(
                            point(
                                column.left(),
                                composer.top() - px((16. * font_ratio).max(26.) + 8.) - px(8.),
                            ),
                            size(column.size.width, px(0.)),
                        )
                    });
            // Reserve the measured column's top edge and the gap above the pill.
            // With a bottom anchor every disclosure grows upward until this
            // real pane/window limit is reached; only then does content scroll.
            let height = anchor_bounds
                .zip(self.chat_column.get())
                .map(|(anchor, column)| {
                    (anchor.top() - px(7.) - column.top().max(px(8.)) - px(8.)).max(px(1.))
                })
                .unwrap_or((window.viewport_size().height - px(80.)).max(px(1.)));
            pill = pill
                .child(
                    Popover::new("working-plan-popover")
                        .appearance(false)
                        .trigger_style(StyleRefinement {
                            flex_shrink: Some(0.),
                            ..Default::default()
                        })
                        .anchor(Anchor::BottomCenter)
                        .when_some(anchor_bounds, |popup, bounds| popup.anchor_bounds(bounds))
                        .offset(px(7.))
                        .open(self.plan_open)
                        .overlay_closable(true)
                        .trigger(
                            Button::new("working-plan-toggle")
                                .compact()
                                .xsmall()
                                .ghost()
                                .h_auto()
                                .px(px(8.))
                                .py(px(3.))
                                .rounded_l_full()
                                .rounded_r_none()
                                .text_size(px(11. * font_ratio))
                                .icon(Icon::new(IconName::FileText))
                                .when(!compact_controls, |button| button.child("Work Plan"))
                                .when(compact_controls, |button| {
                                    button.w(px(28.)).h(px(24.)).px_0().py_0()
                                })
                                .debug_selector(|| "working-plan-toggle".into())
                                .tooltip("Show active work plan")
                                .accessibility_label("Show active work plan"),
                        )
                        .on_open_change(move |open, _, cx| {
                            let _ = owner.update(cx, |bubble, cx| {
                                bubble.plan_open = *open && bubble.active_plan.is_some();
                                cx.notify();
                            });
                        })
                        .content(move |_, _, _| plan_popup(&plan, p, width, height, font_ratio)),
                )
                .child(div().flex_shrink_0().w(px(1.)).h(px(16.)).bg(rgb(p.hairline(0))));
        }
        let pill = pill.child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(10.))
                .h(px(26. * self.font_ratio))
                .child(
                    div()
                        .flex_shrink_0()
                        .child(Spinner::new().xsmall().color(rgb(p.accent).into())),
                )
                .child(
                    div()
                        .id("working-status")
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .debug_selector(|| "working-progress-status".into())
                        .aria_label(status.clone())
                        .child(status_preview),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .debug_selector(|| "working-stop-control".into())
                        .child(
                            Button::new("working-stop")
                                .compact()
                                .xsmall()
                                .ghost()
                                .w(px(18.))
                                .h(px(18.))
                                .text_color(rgb(p.text(2)))
                                .icon(Icon::new(IconName::Square).size(px(9.)))
                                .tooltip(if self.stop_requested {
                                    "Stop requested"
                                } else {
                                    "Stop run"
                                })
                                .accessibility_label("Stop run")
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(WorkingBubbleEvent::Stop)),
                                ),
                        ),
                ),
        );
        ComposerAnchor {
            composer: self.composer.clone(),
            chat_column: self.chat_column.clone(),
            clearance: px(8.),
            flight: 0.,
            child: pill.into_any_element(),
        }
        .into_any_element()
    }
}
/// Anchor above the measured goal/Git/composer stack after cached geometry
/// replays. Clip paint and hit boxes to the chat column beside the sidebar.
pub(crate) struct ComposerAnchor {
    composer: PaintBounds,
    chat_column: PaintBounds,
    child: AnyElement,
    clearance: Pixels,
    flight: f32,
}
impl ComposerAnchor {
    pub(crate) fn new(
        composer: PaintBounds,
        chat_column: PaintBounds,
        child: AnyElement,
        clearance: Pixels,
    ) -> Self {
        Self {
            composer,
            chat_column,
            child,
            clearance,
            flight: 0.,
        }
    }
    pub(crate) fn fly_down(mut self, progress: f32) -> Self {
        self.flight = progress.clamp(0., 1.);
        self
    }
}
impl IntoElement for ComposerAnchor {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for ComposerAnchor {
    type RequestLayoutState = LayoutId;
    type PrepaintState = bool;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let child = self.child.request_layout(window, cx);
        let mut style = Style::default();
        style.position = Position::Absolute;
        (window.request_layout(style, [child], cx), child)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let (Some(composer), Some(column)) = (self.composer.get(), self.chat_column.get()) else {
            return false;
        };
        let child_bounds = window.layout_bounds(*child);
        let size = child_bounds.size;
        if size.width > column.size.width || size.height > column.size.height {
            return false;
        }
        let mut origin = point(
            (composer.origin.x + (composer.size.width - size.width) * 0.5)
                .max(column.origin.x)
                .min(column.origin.x + column.size.width - size.width),
            (composer.origin.y - size.height - self.clearance)
                .min(column.origin.y + column.size.height - size.height - px(8.)),
        );
        // Travel beyond the pane's bottom; its mask clips paint and hit targets.
        origin.y += (column.bottom() + size.height + px(8.) - origin.y) * self.flight;
        // The child already has a layout origin inside its absolute wrapper.
        // Move by the delta, as GPUI Anchored does; prepaint_at would add the
        // existing layout origin a second time and push this below the pane.
        window.with_content_mask(Some(ContentMask::new(column)), |window| {
            window.with_element_offset(origin - child_bounds.origin, |window| {
                self.child.prepaint(window, cx);
            });
        });
        true
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        painted: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *painted {
            if let Some(column) = self.chat_column.get() {
                window.with_content_mask(Some(ContentMask::new(column)), |window| {
                    self.child.paint(window, cx);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::active_work_plan;
    use serde_json::json;

    #[test]
    fn pill_uses_only_the_latest_active_plan_for_its_chat() {
        let snapshot = json!({"workPlans": [
            {"id":"older","taskId":"chat-a","status":"active","updatedAt":1},
            {"id":"closed","taskId":"chat-a","status":"closed","updatedAt":9},
            {"id":"other","taskId":"chat-b","status":"active","updatedAt":10},
            {"id":"current","taskId":"chat-a","status":"active","updatedAt":3}
        ]});
        assert_eq!(
            active_work_plan(&snapshot, "chat-a").unwrap()["id"],
            "current"
        );
        assert_eq!(
            active_work_plan(&snapshot, "chat-b").unwrap()["id"],
            "other"
        );
        assert!(active_work_plan(&snapshot, "chat-c").is_none());
    }

    mod popup_tests {
        use super::super::WorkingBubble;
        use crate::design;
        use gpui::{
            AppContext as _, Bounds, Entity, TestAppContext, WindowAppearance, point, px, size,
        };
        use gpui_kit::component::Root;
        use serde_json::json;
        use std::{
            cell::{Cell, RefCell},
            rc::Rc,
        };

        #[gpui::test]
        fn stop_requests_click_while_starting_and_already_queued(cx: &mut TestAppContext) {
            cx.update(gpui_kit::init);
            let slot = Rc::new(RefCell::new(None::<Entity<WorkingBubble>>));
            let output = slot.clone();
            let (_, visual) = cx.add_window_view(move |window, cx| {
                let column = Rc::new(Cell::new(Some(Bounds::new(
                    point(px(20.), px(20.)),
                    size(px(360.), px(560.)),
                ))));
                let composer = Rc::new(Cell::new(Some(Bounds::new(
                    point(px(20.), px(480.)),
                    size(px(360.), px(100.)),
                ))));
                let palette = design::palette(&json!({}), WindowAppearance::Dark);
                let bubble = cx.new(|_| WorkingBubble::new(composer, column, palette));
                bubble.update(cx, |bubble, cx| {
                    bubble.set_context(true, true, false, palette, &json!({}), "chat", cx)
                });
                *output.borrow_mut() = Some(bubble.clone());
                Root::new(bubble, window, cx)
            });
            let bubble = slot.borrow().as_ref().unwrap().clone();
            let clicks = Rc::new(Cell::new(0));
            let observed = clicks.clone();
            let _subscription = visual.update(|_, cx| {
                cx.subscribe(
                    &bubble,
                    move |_, _: &super::super::WorkingBubbleEvent, _| {
                        observed.set(observed.get() + 1)
                    },
                )
            });
            visual.simulate_resize(size(px(400.), px(600.)));
            for queued in [false, true] {
                visual.update(|_, cx| {
                    bubble.update(cx, |bubble, cx| {
                        bubble.set_context(
                            true,
                            true,
                            queued,
                            bubble.palette,
                            &json!({}),
                            "chat",
                            cx,
                        );
                    })
                });
                visual.update(|window, cx| window.draw(cx).clear(cx));
                let stop = visual
                    .debug_bounds("working-stop-control")
                    .expect("mounted Stop control");
                visual.simulate_click(stop.center(), Default::default());
            }
            assert_eq!(
                clicks.get(),
                2,
                "Stop remains clickable before startup acceptance and while queued"
            );
        }

        #[gpui::test]
        fn progress_status_keeps_stop_reachable_in_narrow_pane(cx: &mut TestAppContext) {
            cx.update(gpui_kit::init);
            let (_, visual) = cx.add_window_view(move |window, cx| {
            let column=Rc::new(Cell::new(Some(Bounds::new(point(px(10.),px(10.)),size(px(240.),px(500.))))));
            let composer=Rc::new(Cell::new(Some(Bounds::new(point(px(10.),px(420.)),size(px(240.),px(80.))))));
            let palette=design::palette(&json!({}),WindowAppearance::Dark);
            let bubble=cx.new(|_|WorkingBubble::new(composer,column,palette));
            bubble.update(cx,|bubble,cx|bubble.set_context(true,false,false,palette,&json!({"events":[{
                "taskId":"chat","kind":"agent_progress","title":"Checking the native layout and keeping the current task controls available while the focused verification runs","createdAt":40
            }]}),"chat",cx));
            Root::new(bubble,window,cx)
        });
            visual.simulate_resize(size(px(260.), px(520.)));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            let status = visual
                .debug_bounds("working-progress-status")
                .expect("compact status mounted");
            let stop = visual
                .debug_bounds("working-stop-control")
                .expect("Stop mounted");
            assert!(status.size.width > px(0.));
            assert!(
                stop.left() >= px(10.) && stop.right() <= px(250.),
                "Stop remains inside pane: {stop:?}"
            );
        }

        #[gpui::test]
        fn design_system_working_bubble_popup_opens_above_pill_and_tracks_plan_lifecycle(
            cx: &mut TestAppContext,
        ) {
            cx.update(gpui_kit::init);
            let slot = Rc::new(RefCell::new(None::<Entity<WorkingBubble>>));
            let output = slot.clone();
            let (_, visual) = cx.add_window_view(move |window, cx| {
            let column = Rc::new(Cell::new(Some(Bounds::new(point(px(20.), px(20.)), size(px(360.), px(560.))))));
            let composer = Rc::new(Cell::new(Some(Bounds::new(point(px(20.), px(480.)), size(px(360.), px(100.))))));
            let palette = design::palette(&json!({}), WindowAppearance::Dark);
            let bubble = cx.new(|_| WorkingBubble::new(composer, column, palette));
            bubble.update(cx, |bubble, cx| bubble.set_context(true, false, false, palette,
                &json!({"workPlans":[{"id":"plan","taskId":"chat","status":"active","title":"Current plan","items":[{"title":"Implement popup","status":"in_progress"}]}]}), "chat", cx));
            *output.borrow_mut() = Some(bubble.clone());
            Root::new(bubble, window, cx)
        });
            visual.simulate_resize(size(px(400.), px(600.)));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            let pill = visual
                .debug_bounds("working-bubble")
                .expect("mounted working pill");
            let trigger = visual
                .debug_bounds("working-plan-toggle")
                .expect("mounted Plan trigger");
            visual.simulate_click(trigger.center(), Default::default());
            visual.update(|window, cx| window.draw(cx).clear(cx));
            let popup = visual
                .debug_bounds("working-plan-popup")
                .expect("open plan popup");
            assert!(
                popup.bottom() <= pill.top(),
                "popup {popup:?} must be above pill {pill:?}"
            );
            assert!(
                (popup.center().x - px(200.)).abs() <= px(1.),
                "popup is centered in pane: {popup:?}"
            );
            assert!(popup.size.width <= px(376.));
            assert!(
                popup.left() >= px(0.) && popup.right() <= px(400.),
                "popup fits window: {popup:?}"
            );
            visual.simulate_click(point(px(390.), px(590.)), Default::default());
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("working-plan-popup").is_none());
            let bubble = slot.borrow().as_ref().unwrap().clone();
            visual.simulate_click(trigger.center(), Default::default());
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("working-plan-popup").is_some());
            visual.update(|_, cx| {
                bubble.update(cx, |bubble, cx| {
                    let palette = bubble.palette;
                    bubble.set_context(
                        true,
                        false,
                        false,
                        palette,
                        &json!({"workPlans":[]}),
                        "chat",
                        cx,
                    );
                })
            });
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("working-plan-popup").is_none());
            assert!(visual.debug_bounds("working-plan-toggle").is_none());
            assert!(visual.debug_bounds("working-bubble").is_some());
        }

        #[gpui::test]
        fn compact_plan_popup_grows_upward_then_scrolls_within_each_pane(cx: &mut TestAppContext) {
            use gpui::{ScrollDelta, ScrollWheelEvent};
            cx.update(gpui_kit::init);
            for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
                for pane_width in [360., 240.] {
                    let case = format!("{appearance:?}, pane width {pane_width}px");
                    let slot = Rc::new(RefCell::new(None::<Entity<WorkingBubble>>));
                    let output = slot.clone();
                    let make_snapshot = |count: usize| {
                        json!({"workPlans":[{
                            "id":"compact-plan", "taskId":"chat", "status":"active", "title":"Current plan",
                            "items": (0..count).map(|index| json!({"id":format!("step-{index}"),
                                "title":format!("Step {index}"), "status":"pending",
                                "note": if index == 0 { "The complete stored response remains available here. ".repeat(4) } else { String::new() }
                            })).collect::<Vec<_>>()
                        }]})
                    };
                    let initial = make_snapshot(4);
                    let (_, visual) = cx.add_window_view(move |window, cx| {
                        let column = Rc::new(Cell::new(Some(Bounds::new(
                            point(px(20.), px(20.)),
                            size(px(pane_width), px(760.)),
                        ))));
                        let composer = Rc::new(Cell::new(Some(Bounds::new(
                            point(px(20.), px(660.)),
                            size(px(pane_width), px(120.)),
                        ))));
                        let palette = design::palette(&json!({}), appearance);
                        let bubble = cx.new(|_| WorkingBubble::new(composer, column, palette));
                        bubble.update(cx, |bubble, cx| {
                            bubble.set_context(true, false, false, palette, &initial, "chat", cx)
                        });
                        *output.borrow_mut() = Some(bubble.clone());
                        Root::new(bubble, window, cx)
                    });
                    visual.simulate_resize(size(px(pane_width + 40.), px(800.)));
                    visual.update(|window, _| window.activate_window());
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                    let pill = visual.debug_bounds("working-bubble").unwrap_or_else(|| {
                        panic!("{case}: working-bubble must fit and paint inside its pane")
                    });
                    assert!(
                        pill.size.width <= px(pane_width),
                        "{case}: pill fits actual pane: {pill:?}"
                    );
                    assert!(
                        pill.left() >= px(20.) && pill.right() <= px(pane_width + 20.),
                        "{case}: pill stays inside column: {pill:?}"
                    );
                    let trigger = visual
                        .debug_bounds("working-plan-toggle")
                        .unwrap_or_else(|| panic!("{case}: working-plan-toggle must be mounted"));
                    let stop = visual
                        .debug_bounds("working-stop-control")
                        .unwrap_or_else(|| panic!("{case}: Stop must remain mounted"));
                    for (name, control) in [("Plan", trigger), ("Stop", stop)] {
                        assert!(
                            control.left() >= pill.left()
                                && control.right() <= pill.right()
                                && control.top() >= pill.top()
                                && control.bottom() <= pill.bottom(),
                            "{case}: {name} hitbox stays inside pill: {control:?} in {pill:?}"
                        );
                    }
                    visual.simulate_click(trigger.center(), Default::default());
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                    let collapsed = visual
                        .debug_bounds("working-plan-popup")
                        .unwrap_or_else(|| panic!("{case}: working-plan-popup must be mounted"));
                    assert!(
                        visual.debug_bounds("work-plan-note-0").is_none(),
                        "{case}: note starts/remains collapsed"
                    );
                    let title = visual
                        .debug_bounds("work-plan-title-0")
                        .unwrap_or_else(|| panic!("{case}: work-plan-title-0 must be mounted"));
                    assert!(
                        title.size.height <= design::ui_scaled_px(17.) + px(1.),
                        "{case}: compact item text uses scaled 17px line height: {title:?}"
                    );
                    let info = visual
                        .debug_bounds("work-plan-info-0")
                        .unwrap_or_else(|| panic!("{case}: work-plan-info-0 must be mounted"));
                    visual.simulate_click(info.center(), Default::default());
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                    let expanded = visual
                        .debug_bounds("working-plan-popup")
                        .unwrap_or_else(|| panic!("{case}: working-plan-popup must be mounted"));
                    assert!(
                        visual.debug_bounds("work-plan-note-0").is_some(),
                        "{case}: Info reveals note"
                    );
                    assert!(
                        (expanded.bottom() - collapsed.bottom()).abs() <= px(1.),
                        "{case}: bottom anchor stays fixed: {collapsed:?} -> {expanded:?}"
                    );
                    assert!(
                        expanded.top() < collapsed.top(),
                        "{case}: disclosure grows upward: {collapsed:?} -> {expanded:?}"
                    );
                    assert!(
                        expanded.top() >= px(20.) && expanded.bottom() < px(660.),
                        "{case}: expanded popup stays above composer and below pane top: {expanded:?}"
                    );
                    assert!(
                        expanded.left() >= px(20.) && expanded.right() <= px(pane_width + 20.),
                        "{case}: expanded popup fits pane width: {expanded:?}"
                    );
                    // Collapse by a second real Info click.
                    let info = visual
                        .debug_bounds("work-plan-info-0")
                        .unwrap_or_else(|| panic!("{case}: work-plan-info-0 must be mounted"));
                    visual.simulate_click(info.center(), Default::default());
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                    assert!(
                        visual.debug_bounds("work-plan-note-0").is_none(),
                        "{case}: note starts/remains collapsed"
                    );
                    let bubble = slot.borrow().as_ref().unwrap().clone();
                    // A taller real plan exceeds the old 340px cap and uses the
                    // whole available height, with wheel access to its last item.
                    let long = make_snapshot(32);
                    visual.update(|_, cx| {
                        bubble.update(cx, |bubble, cx| {
                            bubble.set_context(
                                true,
                                false,
                                false,
                                bubble.palette,
                                &long,
                                "chat",
                                cx,
                            );
                        })
                    });
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                    let popup = visual
                        .debug_bounds("working-plan-popup")
                        .unwrap_or_else(|| panic!("{case}: working-plan-popup must be mounted"));
                    assert!(
                        popup.size.height > px(340.),
                        "{case}: actual pane space replaces the fixed cap: {popup:?}"
                    );
                    assert!(
                        popup.top() >= px(20.) && popup.bottom() <= collapsed.bottom() + px(1.),
                        "{case}: scrolling popup stays in pane: {popup:?}"
                    );
                    let tail_before = visual
                        .debug_bounds("work-plan-step-31")
                        .unwrap_or_else(|| panic!("{case}: work-plan-step-31 must be mounted"));
                    assert!(
                        tail_before.bottom() > popup.bottom(),
                        "{case}: oversized plan must overflow into scroll content"
                    );
                    visual.simulate_event(ScrollWheelEvent {
                        position: popup.center(),
                        delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
                        ..Default::default()
                    });
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                    let tail_after = visual
                        .debug_bounds("work-plan-step-31")
                        .unwrap_or_else(|| panic!("{case}: work-plan-step-31 must be mounted"));
                    assert!(
                        tail_after.top() < tail_before.top(),
                        "{case}: wheel scroll moves the plan, not its anchor"
                    );
                    assert!(
                        tail_after.bottom() <= popup.bottom() && tail_after.top() >= popup.top(),
                        "{case}: last item is reachable: {tail_after:?} in {popup:?}"
                    );
                    assert!(
                        visual
                            .debug_bounds("working-stop-control")
                            .unwrap_or_else(|| panic!(
                                "{case}: working-stop-control must be mounted"
                            ))
                            .top()
                            > popup.bottom(),
                        "{case}: Stop stays reachable beneath the popup"
                    );
                    // Each case owns a real window and transient popup leases.
                    // Dismiss through the mounted outside-click path before
                    // removing the window, so the next case starts independently.
                    visual
                        .simulate_click(point(px(pane_width + 38.), px(798.)), Default::default());
                    visual.update(|window, cx| window.draw(cx).clear(cx));
                    assert!(
                        visual.debug_bounds("working-plan-popup").is_none(),
                        "{case}: outside click dismisses popup before window cleanup"
                    );
                    visual.update(|window, _| window.remove_window());
                    slot.borrow_mut().take();
                }
            }
        }
    }
}

#[cfg(test)]
mod progress_tests {
    use super::task_progress;
    use serde_json::json;
    #[test]
    fn progress_is_task_scoped_and_old_user_turn_status_is_hidden() {
        let mut snapshot = json!({"messages":[{"taskId":"a","role":"user","createdAt":20}],"events":[
            {"taskId":"a","kind":"agent_progress","title":"Old","createdAt":10},
            {"taskId":"b","kind":"agent_progress","title":"Other","createdAt":30}]});
        assert_eq!(task_progress(&snapshot, "a"), None);
        snapshot["events"].as_array_mut().unwrap().push(
            json!({"taskId":"a","kind":"agent_progress","title":"Testing layout","createdAt":40}),
        );
        assert_eq!(
            task_progress(&snapshot, "a").as_deref(),
            Some("Testing layout")
        );
    }
}
