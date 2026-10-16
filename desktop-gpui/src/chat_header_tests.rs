//! Mounted chat-header menu actions stay scoped to their owning dock pane.
use super::ChatPane;
use crate::{design, docking::{DockWorkspace, PaneSpec}};
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext, VisualTestContext, px, size};
use gpui_kit::component::dock::DockPlacement;
use monitter_lib::experimental_desktop::{DesktopService, NativeTileTheme};
use serde_json::json;
use std::rc::Rc;

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn chat_header_icon_opens_owner_menu_without_move_or_overflow_controls(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    for (theme, tile) in [("light", NativeTileTheme::Monitter), ("dark", NativeTileTheme::Tern)] {
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
            let pane = cx.new(|cx| ChatPane::performance_fixture("header-owner".into(), snapshot.clone(), window, cx));
            let dock = cx.new(|cx| DockWorkspace::new("chat-header-icon-menu", Rc::new(|_, _, _| None), window, cx));
            let palette = design::palette(&snapshot["settings"], window.appearance());
            dock.update(cx, |dock, cx| {
                dock.set_pane_appearance(&snapshot["settings"], palette, cx);
                dock.add(PaneSpec {
                    id:"chat:header-owner".into(), title:"Owning chat".into(),
                    focus:pane.read(cx).focus_handle(cx), content:pane.clone().into(),
                    lifecycle:None, closable:true,
                }, DockPlacement::Center, None, window, cx).unwrap();
            });
            retained = Some(pane);
            workspace = Some(dock.clone());
            gpui_kit::component::Root::new(dock, window, cx)
        });
        let pane = retained.unwrap();
        let _dock = workspace.unwrap();
        for width in [280., 650.] {
            visual.simulate_resize(size(px(width), px(600.)));
            draw(visual);
            let header = visual.debug_bounds("chat-pane-header").expect("real chat header");
            let icon = visual.debug_bounds("chat-header-menu-trigger").expect("provider icon is the menu trigger");
            assert!(icon.left() >= header.left() && icon.right() <= header.right());
            assert!(icon.top() >= header.top() && icon.bottom() <= header.bottom());
            assert!(icon.size.width <= design::ui_scaled_px(31.), "icon keeps existing identity size");
            assert!(visual.debug_bounds("dock-pane-move-control").is_none(), "chat header never shows move icon");
            // Only opening the local Rename dialog is allowed; no service call or mutation.
            pane.update(visual, |pane, _| pane.service = Some(DesktopService::disconnected_for_ui_fixture()));
            visual.simulate_click(icon.center(), Default::default());
            draw(visual);
            visual.simulate_keystrokes("down");
            visual.simulate_keystrokes("enter");
            draw(visual);
            pane.read_with(visual, |pane, cx| {
                let action = pane.task_actions.as_ref().expect("icon menu opens existing Rename chat action");
                assert_eq!(action.read(cx).task_id(), "header-owner");
            });
            pane.update(visual, |pane, cx| { pane.task_actions = None; pane.service = None; cx.notify(); });
            visual.simulate_keystrokes("escape");
            draw(visual);
        }
        if tile == NativeTileTheme::Tern {
            let header = visual.debug_bounds("chat-pane-header").unwrap();
            let close = visual.debug_bounds("chat-header-close").expect("close remains available");
            assert!(close.left() >= header.left() && close.right() <= header.right());
        }
    }
    design::set_tile_theme(NativeTileTheme::Monitter);
}
