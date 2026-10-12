//! Regression for the retained sidebar's real divider gesture and its
//! post-release collapse normalization.
use super::{
    expanded_sidebar_width, saved_sidebar_width_target, sidebar_resize_target,
    sidebar_should_collapse, sidebar_split_element,
};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, Modifiers,
    MouseButton, ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext,
    Window, div, point, px,
};
use gpui_kit::component::resizable::{ResizablePanelEvent, ResizableState};
use std::{cell::Cell, rc::Rc};

struct SidebarResizeHarness {
    state: Entity<ResizableState>,
    collapsed: Rc<Cell<bool>>,
    persisted_mode: Rc<Cell<super::preferences_modal::SidebarMode>>,
    width_touched: Rc<Cell<bool>>,
    suppress_resize_event: Rc<Cell<bool>>,
    consume_sibling_mouse_moves: bool,
    width_design: f32,
    scale: u16,
}

impl Render for SidebarResizeHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Match Desktop::render: an active pointer gesture owns the width;
        // normalize only after the Resized event has changed collapse state.
        let (width_design, is_resizing) = self.state.read_with(cx, |state, _| {
            (
                state.sizes().first().map_or_else(
                    || self.width_design,
                    |width| width.as_f32() / super::design::ui_px(1., self.scale).as_f32(),
                ),
                state.is_resizing(),
            )
        });
        if let Some(desired) =
            sidebar_resize_target(width_design, self.collapsed.get(), is_resizing, self.scale)
        {
            self.suppress_resize_event.set(true);
            self.state.update(cx, |state, cx| {
                state.resize_panel(0, super::design::ui_px(desired, self.scale), window, cx);
            });
            self.suppress_resize_event.set(false);
        }

        // Mirror Desktop::apply_saved_sidebar_width: persisted width is
        // re-applied on redraw until the user commits a resize. The production
        // helper must yield while a live pointer gesture owns the panel.
        let (has_panels, current_width, is_resizing) = self.state.read_with(cx, |state, _| {
            (
                state.sizes().len() == 2,
                state.sizes().first().map_or(self.width_design, |width| {
                    width.as_f32() / super::design::ui_px(1., self.scale).as_f32()
                }),
                state.is_resizing(),
            )
        });
        if has_panels {
            if let Some(desired) = saved_sidebar_width_target(
                Some(self.width_design),
                current_width,
                self.width_touched.get(),
                self.collapsed.get(),
                is_resizing,
            ) {
                self.suppress_resize_event.set(true);
                self.state.update(cx, |state, cx| {
                    state.resize_panel(0, super::design::ui_px(desired, self.scale), window, cx);
                });
                self.suppress_resize_event.set(false);
            }
        }

        // Keep the app's root-level release guard in the fixture. It observes
        // pointer moves in capture phase, so the real divider gesture must
        // still reach the nested resizable group while the left button is held.
        let sidebar: AnyElement = div()
            .id("sidebar-panel")
            .size_full()
            .debug_selector(|| "sidebar-panel".into())
            .into_any_element();
        let sibling = div()
            .size_full()
            .debug_selector(|| "sidebar-sibling".into());
        let sibling = if self.consume_sibling_mouse_moves {
            sibling.on_mouse_move(|_, _, cx| cx.stop_propagation())
        } else {
            sibling
        };

        div()
            .w(px(800.))
            .h(px(180.))
            .relative()
            .child(super::pointer_guard::PointerReleaseGuard)
            .child(sidebar_split_element(
                &self.state,
                expanded_sidebar_width(self.width_design),
                self.scale,
                sidebar,
                sibling.into_any_element(),
            ))
    }
}

fn fixture(
    cx: &mut TestAppContext,
    consume_sibling_mouse_moves: bool,
    width_design: f32,
    scale: u16,
) -> (Entity<SidebarResizeHarness>, &mut VisualTestContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let state = cx.update(|cx| cx.new(|_| ResizableState::default()));
    let collapsed = Rc::new(Cell::new(false));
    let persisted_mode = Rc::new(Cell::new(super::preferences_modal::SidebarMode::Full));
    let mode_for_event = persisted_mode.clone();
    let width_touched = Rc::new(Cell::new(false));
    let suppress_resize_event = Rc::new(Cell::new(false));
    let collapsed_for_event = collapsed.clone();
    let suppress_for_event = suppress_resize_event.clone();
    let touched_for_event = width_touched.clone();
    let resize_state = state.clone();
    let (harness, visual) = cx.add_window_view(move |_, cx| {
        cx.subscribe(&resize_state, move |harness, state, event, cx| {
            if !matches!(event, ResizablePanelEvent::Resized) || suppress_for_event.get() {
                return;
            }
            // Production persists the width only after the resize gesture
            // ends; intermediate redraws must continue to be protected by the
            // ResizableState::is_resizing() guard.
            if state.read(cx).is_resizing() {
                return;
            }
            let Some(width) = state.read(cx).sizes().first().map(|width| width.as_f32()) else {
                return;
            };
            touched_for_event.set(true);
            collapsed_for_event.set(sidebar_should_collapse(
                width / super::design::ui_px(1., scale).as_f32(),
                collapsed_for_event.get(),
                scale,
            ));
            // Mirror the production release write without touching a profile.
            mode_for_event.set(if collapsed_for_event.get() { super::preferences_modal::SidebarMode::Icons } else { super::preferences_modal::SidebarMode::Full });
            // Desktop::queue_preferences_save also calls cx.notify(), making
            // the release-driven rail state visible immediately.
            cx.notify();
            let _ = harness;
        })
        .detach();
        SidebarResizeHarness {
            state: state.clone(),
            collapsed: collapsed.clone(),
            persisted_mode: persisted_mode.clone(),
            width_touched: width_touched.clone(),
            suppress_resize_event: suppress_resize_event.clone(),
            consume_sibling_mouse_moves,
            width_design,
            scale,
        }
    });
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.update(|window, cx| window.draw(cx).clear(cx));
    (harness, visual)
}

fn drag_sidebar_to(visual: &mut VisualTestContext, target_x: f32) {
    let boundary = visual
        .debug_bounds("sidebar-sibling")
        .expect("sidebar sibling should have layout bounds")
        .left()
        .as_f32();
    let start = point(px(boundary - 2.), px(90.));
    visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    // The first move beyond the handle activates GPUI's DragPanel payload and
    // sets `resizing_panel_ix`; the resize-group's window-level mouse handler
    // consumes subsequent held moves. Match the pinned resizable harness,
    // which sends a second move after drag activation.
    visual.simulate_mouse_move(
        point(px(boundary + 10.), px(90.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_move(
        point(px(target_x), px(90.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn sidebar_drag_crosses_threshold_then_release_collapses_and_can_expand(cx: &mut TestAppContext) {
    let (harness, visual) = fixture(cx, false, 252., 100);

    drag_sidebar_to(visual, 120.);
    harness.read_with(visual, |harness, cx| {
        assert!(harness.state.read(cx).is_resizing());
        assert!(!harness.collapsed.get());
        assert_eq!(
            harness.state.read(cx).sizes()[0].as_f32().round(),
            120.,
            "the active divider drag must be allowed below the normal 160px minimum"
        );
        assert_eq!(sidebar_resize_target(120., false, true, 100), None);
    });
    visual.simulate_mouse_up(
        point(px(120.), px(90.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.update(|window, cx| window.draw(cx).clear(cx));
    harness.read_with(visual, |harness, cx| {
        assert!(
            harness.collapsed.get(),
            "release below 140px collapses the rail"
        );
        assert!(!harness.state.read(cx).is_resizing());
        assert_eq!(
            harness.state.read(cx).sizes()[0].as_f32().round(),
            80.,
            "the release frame normalizes the collapsed rail to its compact width"
        );
    });

    drag_sidebar_to(visual, 105.);
    harness.read_with(visual, |harness, cx| {
        assert!(harness.state.read(cx).is_resizing());
        assert!(harness.collapsed.get());
        assert_eq!(harness.state.read(cx).sizes()[0].as_f32().round(), 105.);
    });
    visual.simulate_mouse_up(
        point(px(105.), px(90.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.update(|window, cx| window.draw(cx).clear(cx));
    harness.read_with(visual, |harness, cx| {
        assert!(
            !harness.collapsed.get(),
            "dragging past the compact hysteresis threshold expands the rail"
        );
        assert_eq!(
            harness.state.read(cx).sizes()[0].as_f32().round(),
            160.,
            "the expanded rail restores its ordinary minimum width after release"
        );
    });
}

#[gpui_kit::test]
fn sidebar_resize_owns_held_moves_over_a_child_that_stops_bubbling(cx: &mut TestAppContext) {
    let (harness, visual) = fixture(cx, true, 252., 100);
    let boundary = visual
        .debug_bounds("sidebar-sibling")
        .expect("sidebar sibling should have layout bounds")
        .left()
        .as_f32();

    visual.simulate_mouse_down(
        point(px(boundary - 2.), px(90.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.simulate_mouse_move(
        point(px(boundary + 10.), px(90.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_move(
        point(px(340.), px(90.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));

    harness.read_with(visual, |harness, cx| {
        assert!(harness.state.read(cx).is_resizing());
        assert_eq!(
            harness.state.read(cx).sizes()[0].as_f32().round(),
            340.,
            "capture-phase splitter ownership must survive a child consuming held moves"
        );
    });

    visual.simulate_mouse_up(
        point(px(340.), px(90.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));
    harness.read_with(visual, |harness, cx| {
        assert!(!harness.state.read(cx).is_resizing());
    });
}

#[gpui_kit::test]
fn production_sidebar_builder_scales_and_clamps_stale_saved_width(cx: &mut TestAppContext) {
    const SCALE: u16 = 135;
    let stale_saved_width = 557.9226;
    let clamped = expanded_sidebar_width(stale_saved_width);
    assert_eq!(
        clamped, 360.,
        "stale preference is clamped before the panel builder"
    );

    let (harness, visual) = fixture(cx, false, stale_saved_width, SCALE);
    let seam = visual
        .debug_bounds("sidebar-sibling")
        .expect("production builder lays out the content panel")
        .left()
        .as_f32();
    assert!((seam - super::design::ui_px(360., SCALE).as_f32()).abs() < 1.);
    harness.read_with(visual, |harness, cx| {
        let actual = harness.state.read(cx).sizes()[0].as_f32();
        assert!((actual - super::design::ui_px(360., SCALE).as_f32()).abs() < 1.);
    });
}

#[gpui_kit::test]
fn scaled_production_divider_supports_repeat_drags_and_clamps_at_360_design_px(
    cx: &mut TestAppContext,
) {
    const SCALE: u16 = 135;
    let (harness, visual) = fixture(cx, false, 237.46, SCALE);
    let initial_seam = visual
        .debug_bounds("sidebar-sibling")
        .expect("production content panel is laid out")
        .left()
        .as_f32();
    assert!((initial_seam - 320.571).abs() < 1.);

    // The press starts two physical pixels inside the sidebar edge, within
    // GPUI's native nine-pixel band straddling the rendered seam.
    drag_sidebar_to(visual, 300.);
    harness.read_with(visual, |harness, cx| {
        assert!(harness.state.read(cx).is_resizing());
        assert!((harness.state.read(cx).sizes()[0].as_f32() - 300.).abs() < 1.);
    });
    visual.simulate_mouse_up(
        point(px(300.), px(90.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));

    // A second full drag from the newly rendered seam reaches the configured
    // max of 360 design pixels (486 physical at 135%), then commits on release.
    drag_sidebar_to(visual, 650.);
    harness.read_with(visual, |harness, cx| {
        assert!(harness.state.read(cx).is_resizing());
        assert!((harness.state.read(cx).sizes()[0].as_f32() - 486.).abs() < 1.);
    });
    visual.simulate_mouse_up(
        point(px(650.), px(90.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));
    harness.read_with(visual, |harness, cx| {
        assert!(!harness.state.read(cx).is_resizing());
        let width_design =
            harness.state.read(cx).sizes()[0].as_f32() / super::design::ui_px(1., SCALE).as_f32();
        assert!((width_design - 360.).abs() < 1.);
    });
}

#[gpui_kit::test]
fn saved_width_redraw_does_not_cancel_a_pointer_owned_resize(cx: &mut TestAppContext) {
    const SCALE: u16 = 135;
    let saved_width = 237.46;
    let (harness, visual) = fixture(cx, false, saved_width, SCALE);
    let seam = visual
        .debug_bounds("sidebar-sibling")
        .expect("production content panel is laid out")
        .left()
        .as_f32();
    assert!((seam - super::design::ui_px(saved_width, SCALE).as_f32()).abs() < 1.);

    let origin = point(px(seam - 2.), px(90.));
    visual.simulate_mouse_down(origin, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        point(px(seam + 10.), px(90.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_move(
        point(px(seam + 35.), px(90.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );

    // This draw is the production failure trigger: Desktop reapplies its
    // still-untouched saved width while the mouse button remains held.
    visual.update(|window, cx| window.draw(cx).clear(cx));
    harness.read_with(visual, |harness, cx| {
        let state = harness.state.read(cx);
        assert!(
            state.is_resizing(),
            "saved-width hydration must preserve gesture ownership across redraw"
        );
        assert!(
            (state.sizes()[0].as_f32() - (seam + 35.)).abs() < 2.,
            "the live drag width should survive render-time saved-width hydration"
        );
        assert!(
            !harness.width_touched.get(),
            "the saved preference remains untouched before release"
        );
    });

    visual.simulate_mouse_move(
        point(px(seam + 60.), px(90.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));
    harness.read_with(visual, |harness, cx| {
        assert!(
            harness.state.read(cx).is_resizing(),
            "a second move after redraw still belongs to the gesture"
        );
        assert!((harness.state.read(cx).sizes()[0].as_f32() - (seam + 60.)).abs() < 2.);
    });

    visual.simulate_mouse_up(
        point(px(seam + 60.), px(90.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));
    harness.read_with(visual, |harness, cx| {
        assert!(!harness.state.read(cx).is_resizing());
        assert!(
            harness.width_touched.get(),
            "release commits the user's new width"
        );
        assert!((harness.state.read(cx).sizes()[0].as_f32() - (seam + 60.)).abs() < 2.);
    });
}

#[gpui_kit::test]
fn sidebar_layout_ticks_are_silent_and_live_drag_wins(cx: &mut TestAppContext) {
    let (harness, visual) = fixture(cx, false, 252., 100);
    let events = Rc::new(Cell::new(0));
    let capture = events.clone();
    let state = harness.read_with(visual, |harness, _| harness.state.clone());
    let _subscription = visual.update(|window, cx| window.subscribe(&state, cx, move |_, _, _, _| {
        capture.set(capture.get() + 1);
    }));
    // The toggle owns its target, so saved-width hydration cannot undo it.
    harness.update(visual, |harness, _| harness.width_touched.set(true));
    visual.update(|window, cx| {
        assert!(state.update(cx, |state, cx| state.resize_panel_for_layout(0, px(190.), window, cx)));
        window.draw(cx).clear(cx);
    });
    assert_eq!(events.get(), 0, "a geometry tick must not emit a persistence event");
    drag_sidebar_to(visual, 225.);
    visual.update(|window, cx| {
        let before = state.read(cx).sizes().clone();
        assert!(state.read(cx).is_resizing());
        assert!(!state.update(cx, |state, cx| state.resize_panel_for_layout(0, px(100.), window, cx)));
        assert_eq!(state.read(cx).sizes(), &before);
        assert!(state.read(cx).is_resizing(), "layout interpolation cannot end a live pointer gesture");
    });
    visual.simulate_mouse_up(point(px(225.), px(90.)), MouseButton::Left, Modifiers::default());
}


#[gpui_kit::test]
fn hidden_transition_drag_release_persists_full_even_without_a_collapse_change(cx: &mut TestAppContext) {
    let (harness, visual) = fixture(cx, false, 252., 100);
    // Icons->Hidden saves Hidden immediately, then pointer takeover clears
    // hidden and leaves collapsed=false. Releasing above the full threshold
    // must persist Full even though collapsed remains false throughout.
    harness.update(visual, |harness, _| {
        harness.persisted_mode.set(super::preferences_modal::SidebarMode::Hidden);
        harness.collapsed.set(false);
        harness.width_touched.set(true);
    });
    drag_sidebar_to(visual, 220.);
    harness.read_with(visual, |harness, _| {
        assert!(!harness.collapsed.get());
        assert_eq!(harness.persisted_mode.get(), super::preferences_modal::SidebarMode::Hidden);
    });
    visual.simulate_mouse_up(point(px(220.), px(90.)), MouseButton::Left, Modifiers::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    harness.read_with(visual, |harness, _| {
        assert!(!harness.collapsed.get());
        assert_eq!(harness.persisted_mode.get(), super::preferences_modal::SidebarMode::Full);
    });
}
