//! Mounted chat-header menu actions stay scoped to their owning dock pane.
use super::ChatPane;
use crate::{
    design,
    docking::{DockWorkspace, PaneSpec},
};
use gpui_kit::component::dock::DockPlacement;
use gpui_kit::{
    AppContext as _, Focusable as _, TestAppContext, VisualTestContext, point, px, size,
};
use monitter_lib::experimental_desktop::{DesktopService, NativeTileTheme};
use serde_json::json;
use std::rc::Rc;

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn chat_header_and_watcher_chrome_stay_scoped_to_the_owning_pane(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    for (theme, tile) in [
        ("light", NativeTileTheme::Monitter),
        ("dark", NativeTileTheme::Tern),
    ] {
        design::set_tile_theme(tile);
        let snapshot = Rc::new(json!({
            "settings":{"theme":theme,"interfaceDensity":"normal","nativeTileTheme":if tile == NativeTileTheme::Tern {"tern"} else {"monitter"}},
            "tasks":[{"id":"header-owner","title":"Owning chat","agentId":"header-agent","provider":"codex","status":"idle","queue":[]}],
            "agents":[{"id":"header-agent","name":"Codex","provider":"codex"}],
            "messages":[],"events":[],"approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]
        }));
        let mut retained = None;
        let mut workspace = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| {
                ChatPane::performance_fixture("header-owner".into(), snapshot.clone(), window, cx)
            });
            let dock = cx.new(|cx| {
                DockWorkspace::new("chat-header-icon-menu", Rc::new(|_, _, _| None), window, cx)
            });
            let palette = design::palette(&snapshot["settings"], window.appearance());
            dock.update(cx, |dock, cx| {
                dock.set_pane_appearance(&snapshot["settings"], palette, cx);
                dock.add(
                    PaneSpec {
                        id: "chat:header-owner".into(),
                        title: "Owning chat".into(),
                        focus: pane.read(cx).focus_handle(cx),
                        content: pane.clone().into(),
                        lifecycle: None,
                        closable: true,
                    },
                    DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            });
            retained = Some(pane);
            workspace = Some(dock.clone());
            gpui_kit::component::Root::new(dock, window, cx)
        });
        let pane = retained.unwrap();
        let _dock = workspace.unwrap();
        visual.update(|window, _| window.activate_window());
        for width in [280., 650.] {
            visual.simulate_resize(size(px(width), px(600.)));
            visual.simulate_mouse_move(point(px(width / 2.), px(580.)), None, Default::default());
            draw(visual);
            let header = visual
                .debug_bounds("chat-pane-header")
                .expect("real chat header");
            let icon = visual
                .debug_bounds("chat-header-menu-trigger")
                .expect("provider icon is the menu trigger");
            let hover_sensor = visual
                .debug_bounds("chat-header-hover-sensor")
                .expect("uncached pane layer owns the Chat header hover target");
            assert_eq!(hover_sensor.top(), header.top());
            assert_eq!(hover_sensor.bottom(), header.bottom());
            assert!(icon.left() >= header.left() && icon.right() <= header.right());
            assert!(icon.top() >= header.top() && icon.bottom() <= header.bottom());
            if tile == NativeTileTheme::Tern {
                let left_inset = icon.left() - header.left();
                let top_inset = icon.top() - header.top();
                assert!((left_inset - top_inset).abs() <= px(1.),
                    "provider control keeps equal top and left insets: {icon:?} in {header:?}");
            }
            assert!(
                icon.size.width <= design::ui_scaled_px(31.),
                "icon keeps existing identity size"
            );
            assert!(
                visual.debug_bounds("dock-pane-move-control").is_none(),
                "chat header never shows move icon"
            );
            let idle_controls = visual
                .debug_bounds("chat-header-controls")
                .expect("chat controls stay mounted while collapsed");
            assert_eq!(
                idle_controls.size.width.as_f32().round(),
                0.,
                "chat controls stay hidden until header hover"
            );
            visual.simulate_mouse_move(hover_sensor.center(), None, Default::default());
            draw(visual);
            pane.read_with(visual, |pane, _| {
                assert!(
                    pane.header_controls_hovered,
                    "the uncached header sensor publishes its hover state"
                );
            });
            let visible_controls = visual
                .debug_bounds("chat-header-controls")
                .expect("chat controls stay mounted on header hover");
            assert!(
                visible_controls.size.width.as_f32() > 20.,
                "header hover expands the chat control strip"
            );
            let sidebar = visual
                .debug_bounds("toggle-pane-secondary-sidebar")
                .expect("header hover reveals the secondary-sidebar control");
            assert!(sidebar.left() >= header.left() && sidebar.right() <= header.right());
            if tile == NativeTileTheme::Tern {
                let close = visual
                    .debug_bounds("chat-header-close")
                    .expect("header hover reveals the close control");
                assert!(close.left() >= header.left() && close.right() <= header.right());
            }
            // Only opening the local Rename dialog is allowed; no service call or mutation.
            pane.update(visual, |pane, _| {
                pane.service = Some(DesktopService::disconnected_for_ui_fixture())
            });
            visual.simulate_click(icon.center(), Default::default());
            draw(visual);
            visual.simulate_keystrokes("down");
            visual.simulate_keystrokes("enter");
            draw(visual);
            pane.read_with(visual, |pane, cx| {
                let action = pane
                    .task_actions
                    .as_ref()
                    .expect("icon menu opens existing Rename chat action");
                assert_eq!(action.read(cx).task_id(), "header-owner");
            });
            pane.update(visual, |pane, cx| {
                pane.task_actions = None;
                pane.service = None;
                cx.notify();
            });
            visual.simulate_keystrokes("escape");
            draw(visual);
        }
        if tile == NativeTileTheme::Tern {
            visual.simulate_mouse_move(point(px(325.), px(580.)), None, Default::default());
            draw(visual);
            assert_eq!(
                visual
                    .debug_bounds("chat-header-controls")
                    .unwrap()
                    .size
                    .width
                    .as_f32()
                    .round(),
                0.,
                "leaving the Chat header collapses its controls"
            );
            pane.update(visual, |pane, cx| {
                pane.watcher.enabled = true;
                cx.notify();
            });
            draw(visual);
            let header = visual.debug_bounds("chat-pane-header").unwrap();
            assert!(
                visual.debug_bounds("chat-header-menu-trigger").is_none(),
                "Watch removes the pane identity icon"
            );
            assert!(
                visual
                    .debug_bounds("toggle-pane-secondary-sidebar")
                    .is_none(),
                "Watch removes the secondary-sidebar control"
            );
            assert_eq!(
                visual
                    .debug_bounds("watch-header-controls")
                    .unwrap()
                    .size
                    .width
                    .as_f32()
                    .round(),
                0.,
                "the Watch close control is idle until the header is hovered"
            );
            visual.simulate_mouse_move(header.center(), None, Default::default());
            draw(visual);
            assert!(
                visual
                    .debug_bounds("watch-header-controls")
                    .unwrap()
                    .size
                    .width
                    .as_f32()
                    > 20.,
                "hover expands the Watch close-control strip"
            );
            let close = visual
                .debug_bounds("watch-header-close")
                .expect("hover reveals the Watch close control");
            assert!(close.left() >= header.left() && close.right() <= header.right());
        }
    }
    design::set_tile_theme(NativeTileTheme::Monitter);
}
