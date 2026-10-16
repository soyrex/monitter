//! Mounted regression coverage for the real workspace tabs and shared feed.
use crate::{Desktop, design};
use gpui_kit::{prelude::*, *, component::{Root, Theme, ThemeMode}};
use ::core::prelude::v1::test;
use monitter_lib::experimental_desktop::{NativePreferences, NativeWorkspace, NativeWorkspaceSet};
use serde_json::{Value, json};

struct Tabs(Entity<Desktop>);
impl Render for Tabs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0.update(cx, |view, cx| {
            let palette = design::palette(&view.snapshot["settings"], window.appearance());
            div().size_full().bg(rgb(palette.paper))
                .child(view.render_workspace_tabs(palette, window, cx))
        })
    }
}
fn layout(task: &str) -> Value {
    json!({"version":1,"center":{"panel_name":"TabPanel","children":[{
        "panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":format!("chat:{task}")}}
    }],"info":{"tabs":{"active_index":0}}}})
}
fn mounted(cx: &mut TestAppContext, light: bool) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
    });
    let mut desktop = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(window, cx);
            view.preferences = Some(NativePreferences::default());
            view.preferences_busy = true; // Offline fixture: never persist layout/preferences.
            view.snapshot = json!({"settings":{"theme":if light {"light"} else {"dark"}},
                "agents":[{"id":"agent","name":"Codex"}],
                "tasks":[{"id":"current","agentId":"agent","updatedAt":1},
                    {"id":"other","agentId":"agent","updatedAt":1},
                    {"id":"orphan","agentId":"agent","updatedAt":1}],
                "messages":[],"channels":[],
                "approvalRequests":[{"id":"approval","taskId":"other","status":"pending","summary":"Review this request","createdAt":2},
                    {"id":"global","taskId":"orphan","status":"pending","summary":"Global only","createdAt":3}]}).into();
            view.notification_model.sync(&view.snapshot);
            std::rc::Rc::make_mut(&mut view.snapshot)["messages"] = json!([{"id":"answer","taskId":"other","role":"assistant","text":"Finished the workspace change.","createdAt":20,"phase":"final_answer","streamStatus":"complete"}]);
            view.notification_model.sync(&view.snapshot);
            view.named_workspaces = Some(NativeWorkspaceSet {active_id:"current".into(),items:[("current","Now"),("other","Later"),("empty","Empty")].into_iter().map(|(id,name)|NativeWorkspace {
                id:id.into(),name:name.into(),icon:"folder".into(),color:"#38BFD2".into(),url:None,layout:Some(layout(id)),
            }).collect()});
            view
        });
        desktop = Some(view.clone());
        Root::new(cx.new(|_| Tabs(view)), window, cx)
    });
    let desktop = desktop.unwrap();
    for (width, font) in [(1000.,14.),(600.,20.)] {
        visual.simulate_resize(size(px(width),px(650.)));
        visual.update(|window,cx| {
            Theme::update(cx, |theme| theme.font_size=px(font));
            window.draw(cx).clear(cx);
        });
        let pill = visual.debug_bounds("workspace-notifications-other").expect("inactive workspace pill");
        assert_eq!(pill.size.width, pill.size.height, "notification badge stays circular");
        assert_eq!(pill.size.height, crate::design::ui_scaled_px(16.), "small badge does not grow with plan/interface text size");
        assert!(visual.debug_bounds("workspace-notifications-current").is_none());
        assert!(visual.debug_bounds("workspace-notifications-empty").is_none());
        visual.simulate_click(pill.center(), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window,cx|window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("notification-row-approval:approval").is_some());
        assert!(visual.debug_bounds("notification-row-task:other").is_some());
        assert!(visual.debug_bounds("notification-row-approval:global").is_none());
        visual.read(|cx| {
            let view = desktop.read(cx);
            assert_eq!(view.named_workspaces.as_ref().unwrap().active_id,"current", "pill click must not switch workspace");
            assert!(view.workspace_editor.is_none(), "pill must not open tab editor");
            assert!(view.workspace_switch.is_none());
        });
        visual.simulate_click(point(px(20.),px(550.)), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window,cx|window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("notification-list").is_none());
    }
    let tab_before=visual.debug_bounds("workspace-tab-other").unwrap();
    let following_before=visual.debug_bounds("workspace-tab-empty").unwrap();
    // Dismiss an alert through the workspace feed; the global model loses that same entry.
    let pill=visual.debug_bounds("workspace-notifications-other").unwrap();
    visual.simulate_click(pill.center(), Modifiers::none());visual.run_until_parked();
    visual.update(|window,cx|window.draw(cx).clear(cx));
    let dismiss=visual.debug_bounds("notification-dismiss-task:other").unwrap();
    visual.simulate_click(dismiss.center(), Modifiers::none());visual.run_until_parked();
    visual.update(|window,cx|window.draw(cx).clear(cx));
    visual.read(|cx| {
        let entries=desktop.read(cx).notification_model.entries(&desktop.read(cx).snapshot);
        assert_eq!(entries.len(),2);
        assert!(!entries.iter().any(|entry|entry.id=="task:other"));
    });
    assert!(visual.debug_bounds("notification-row-task:other").is_none());
    assert!(visual.debug_bounds("notification-row-approval:approval").is_some(), "pending approval remains");
    // Authoritative removal closes the empty scoped feed and removes its pill.
    visual.update(|_,cx|desktop.update(cx,|view,cx| {
        std::rc::Rc::make_mut(&mut view.snapshot)["approvalRequests"][0]["status"]=json!("approved");
        view.sync_notification_bell(cx);cx.notify();
    }));
    visual.run_until_parked();visual.update(|window,cx|window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("workspace-notifications-other").is_none());
    assert!(visual.debug_bounds("notification-list").is_none());
    assert_eq!(visual.debug_bounds("workspace-tab-other").unwrap(),tab_before,"removing the pill preserves tab bounds");
    assert_eq!(visual.debug_bounds("workspace-tab-empty").unwrap(),following_before,"following workspace tab does not shift");
}
#[gpui_kit::test]
fn workspace_notification_pills_light(cx: &mut TestAppContext) { mounted(cx,true); }
#[gpui_kit::test]
fn workspace_notification_pills_dark(cx: &mut TestAppContext) { mounted(cx,false); }
