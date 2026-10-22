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
fn stack_axes(value: &Value, axes: &mut Vec<u64>) {
    if value["panel_name"] == "StackPanel" {
        if let Some(axis) = value["info"]["stack"]["axis"].as_u64() {
            axes.push(axis);
        }
    }
    if let Some(children) = value["children"].as_array() {
        for child in children {
            stack_axes(child, axes);
        }
    }
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
                "tasks":[{"id":"current","agentId":"agent","status":"idle","updatedAt":1},
                    {"id":"other","agentId":"agent","status":"running","updatedAt":1},
                    {"id":"orphan","agentId":"agent","status":"running","updatedAt":1}],
                "messages":[],"channels":[],
                "approvalRequests":[{"id":"current-approval","taskId":"current","status":"pending","summary":"Current workspace request","createdAt":1},
                    {"id":"approval","taskId":"other","status":"pending","summary":"Review this request","createdAt":2},
                    {"id":"global","taskId":"orphan","status":"pending","summary":"Global only","createdAt":3}]}).into();
            view.notification_model.sync(&view.snapshot);
            std::rc::Rc::make_mut(&mut view.snapshot)["messages"] = json!([{"id":"answer","taskId":"other","role":"assistant","text":"Finished the workspace change.","createdAt":20,"phase":"final_answer","streamStatus":"complete"}]);
            view.notification_model.sync(&view.snapshot);
            view.named_workspaces = Some(NativeWorkspaceSet {active_id:"current".into(),items:[("current","Now"),("other","Later"),("empty","Empty")].into_iter().map(|(id,name)|NativeWorkspace {
                id:id.into(),name:name.into(),icon:"folder".into(),color:"#38BFD2".into(),url:None,layout:Some(layout(id)),
            }).collect()});
            view.window.dock.update(cx, |dock, cx| {
                dock.apply_layout_preset("single", window, cx).unwrap();
                assert!(dock.focus("empty:preset-1", window, cx));
            });
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
        assert!(visual.debug_bounds("workspace-notifications-current").is_some(),
            "selected workspace keeps its nonzero notification pill visible");
        assert!(visual.debug_bounds("workspace-notifications-empty").is_none());
        let spinner = visual.debug_bounds("workspace-active-spinner-other")
            .expect("inactive workspace with a running task has an activity spinner");
        assert_eq!(spinner.size, size(px(10.), px(10.)), "activity spinner is 10x10");
        let icon = visual.debug_bounds("workspace-edit-icon-other")
            .expect("workspace icon anchors the activity spinner");
        assert!(spinner.center().x > icon.center().x && spinner.center().y < icon.center().y,
            "activity spinner stays on the icon's top-right corner: icon={icon:?}, spinner={spinner:?}");
        assert!(visual.debug_bounds("workspace-active-spinner-current").is_none());
        assert!(visual.debug_bounds("workspace-active-spinner-empty").is_none());
        if width == 1000. {
            let tabs = visual.debug_bounds("named-workspace-tabs").expect("centered workspace tabs");
            assert!((tabs.center().x - px(width / 2.)).abs() <= px(1.),
                "workspace tabs are centered on the full bar: {tabs:?}");
            assert!(visual.debug_bounds("split-workspace-vertical").is_some());
            assert!(visual.debug_bounds("split-workspace-horizontal").is_some());
            assert!(visual.debug_bounds("add-workspace").is_some());
        }
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
    visual.update(|_,cx|desktop.update(cx,|view,cx| {
        std::rc::Rc::make_mut(&mut view.snapshot)["tasks"][1]["status"]=json!("idle");
        cx.notify();
    }));
    visual.run_until_parked();visual.update(|window,cx|window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("workspace-active-spinner-other").is_none(),
        "spinner disappears when the workspace task stops running");
    assert_eq!(visual.debug_bounds("workspace-tab-other").unwrap(),tab_before,
        "removing the spinner preserves tab bounds");
    assert_eq!(visual.debug_bounds("workspace-tab-empty").unwrap(),following_before,
        "spinner removal does not shift following workspace tabs");
    // Dismiss an alert through the workspace feed; the global model loses that same entry.
    let pill=visual.debug_bounds("workspace-notifications-other").unwrap();
    visual.simulate_click(pill.center(), Modifiers::none());visual.run_until_parked();
    visual.update(|window,cx|window.draw(cx).clear(cx));
    let dismiss=visual.debug_bounds("notification-dismiss-task:other").unwrap();
    visual.simulate_click(dismiss.center(), Modifiers::none());visual.run_until_parked();
    visual.update(|window,cx|window.draw(cx).clear(cx));
    visual.read(|cx| {
        let entries=desktop.read(cx).notification_model.entries(&desktop.read(cx).snapshot);
        assert_eq!(entries.len(),3);
        assert!(!entries.iter().any(|entry|entry.id=="task:other"));
    });
    assert!(visual.debug_bounds("notification-row-task:other").is_none());
    assert!(visual.debug_bounds("notification-row-approval:approval").is_some(), "pending approval remains");
    // Authoritative removal closes the empty scoped feed and removes its pill.
    visual.update(|_,cx|desktop.update(cx,|view,cx| {
        std::rc::Rc::make_mut(&mut view.snapshot)["approvalRequests"][1]["status"]=json!("approved");
        view.sync_notification_bell(cx);cx.notify();
    }));
    visual.run_until_parked();visual.update(|window,cx|window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("workspace-notifications-other").is_none());
    assert!(visual.debug_bounds("workspace-notifications-current").is_some(),
        "unrelated active-workspace notifications remain visible");
    assert!(visual.debug_bounds("notification-list").is_none());
    assert_eq!(visual.debug_bounds("workspace-tab-other").unwrap(),tab_before,"removing the pill preserves tab bounds");
    assert_eq!(visual.debug_bounds("workspace-tab-empty").unwrap(),following_before,"following workspace tab does not shift");

    // These controls live inside the native title-bar drag surface. Exercise
    // their real pointer path, not only their accessibility action, so a
    // bubbling mouse-down cannot silently turn the click into a window drag.
    visual.simulate_resize(size(px(1000.), px(650.)));
    visual.update(|window, cx| {
        desktop.update(cx, |view, cx| {
            assert!(view
                .window
                .dock
                .update(cx, |dock, cx| dock.focus("empty:preset-1", window, cx)));
        });
        window.draw(cx).clear(cx);
    });
    let vertical = visual.debug_bounds("split-workspace-vertical").unwrap();
    visual.simulate_mouse_move(vertical.center(), None, Modifiers::none());
    visual.simulate_click(vertical.center(), Modifiers::none());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.read(|cx| {
        let view = desktop.read(cx);
        let dock = view.window.dock.read(cx);
        assert!(
            view.window.error.is_none(),
            "vertical split error: {:?}; current pane: {:?}",
            view.window.error,
            view.window.dock_current
        );
        let layout = dock.serialize(cx).unwrap();
        let mut axes = Vec::new();
        stack_axes(&layout["center"], &mut axes);
        assert_eq!(
            crate::docking::DockWorkspace::saved_ids(&layout)
                .unwrap()
                .len(),
            2,
            "vertical split button inserts a pane");
        assert!(axes.contains(&0),
            "vertical split button creates side-by-side panes");
    });
    let horizontal = visual.debug_bounds("split-workspace-horizontal").unwrap();
    // A real pointer necessarily moves between the two controls. Model that
    // transition so the first control's retained hover/tooltip surface leaves
    // the dispatch path before the second press.
    visual.simulate_mouse_move(horizontal.center(), None, Modifiers::none());
    visual.simulate_click(horizontal.center(), Modifiers::none());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.read(|cx| {
        let view = desktop.read(cx);
        let dock = view.window.dock.read(cx);
        assert!(
            view.window.error.is_none(),
            "horizontal split error: {:?}; current pane: {:?}",
            view.window.error,
            view.window.dock_current
        );
        let layout = dock.serialize(cx).unwrap();
        let mut axes = Vec::new();
        stack_axes(&layout["center"], &mut axes);
        assert_eq!(
            crate::docking::DockWorkspace::saved_ids(&layout)
                .unwrap()
                .len(),
            3,
            "horizontal split button inserts a pane; current={:?}; vertical={vertical:?}; horizontal={horizontal:?}; layout={layout}",
            view.window.dock_current
        );
        assert!(axes.contains(&1), "horizontal split button creates stacked panes");
    });
}
#[gpui_kit::test]
fn workspace_notification_pills_light(cx: &mut TestAppContext) { mounted(cx,true); }
#[gpui_kit::test]
fn workspace_notification_pills_dark(cx: &mut TestAppContext) { mounted(cx,false); }
