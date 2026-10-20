//! Switching saved layouts hides retained panes without closing their lifecycle.
use super::{DockWorkspace, LifecycleHook, PaneFactory, PaneLifecycle, PaneSpec};
use gpui::{
    AppContext as _, Context, IntoElement, Render, Styled as _, TestAppContext, Window, div,
};
use gpui_kit::component::dock::DockPlacement;
use std::{cell::RefCell, rc::Rc};

struct PaneContent;
impl Render for PaneContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

fn spec(id: &str, lifecycle: LifecycleHook, cx: &mut Context<DockWorkspace>) -> PaneSpec {
    PaneSpec {
        id: id.to_owned(),
        title: id.to_owned().into(),
        content: cx.new(|_| PaneContent).into(),
        focus: cx.focus_handle(),
        lifecycle: Some(lifecycle),
        closable: true,
    }
}

#[gpui_kit::test]
fn palette_layout_presets_keep_native_panels_focus_and_lifecycles(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut dock = None;
    let events = Rc::new(RefCell::new(Vec::<PaneLifecycle>::new()));
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let entity = cx.new(|cx| DockWorkspace::new("preset-test", factory, window, cx));
        dock = Some(entity.clone());
        gpui_kit::component::Root::new(entity, window, cx)
    });
    let dock = dock.unwrap();
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        let capture = events.clone();
        let lifecycle: LifecycleHook = Rc::new(move |event, _, _| capture.borrow_mut().push(event));
        for id in ["chat:first", "chat:second", "terminal:retained"] {
            dock.add(spec(id, lifecycle.clone(), cx), DockPlacement::Center, None, window, cx).unwrap();
        }
        dock.focus("chat:second", window, cx);
        let original = dock.panels.borrow().iter().map(|(id,panel)| (id.clone(),panel.entity_id())).collect::<Vec<_>>();
        for (preset, count) in [("columns",2), ("grid",4), ("grid-3x2",6), ("grid-4x2",8), ("columns-3",3), ("columns-4",4), ("single",1)] {
            dock.apply_layout_preset(preset, window, cx).unwrap();
            let area = dock.area.read(cx);
            assert_eq!(super::groups_in(area.layout(DockPlacement::Center).unwrap().root()).len(), count);
            assert_eq!(dock.focused_pane_id(window, cx).as_deref(), Some("chat:second"));
            for (id,key) in &original { assert_eq!(dock.panels.borrow()[id].entity_id(), *key); }
            let ids = DockWorkspace::saved_ids(&dock.serialize(cx).unwrap()).unwrap();
            for (id,_) in &original { assert!(ids.contains(id)); }
            assert!(!events.borrow().iter().any(|event| matches!(event, PaneLifecycle::Removed)));
        }
        let before = dock.serialize(cx).unwrap();
        assert!(dock.apply_layout_preset("bad", window, cx).is_err());
        assert_eq!(dock.serialize(cx).unwrap(), before);
    }));
}

#[gpui_kit::test]
fn saved_layout_restore_retains_pane_identity_until_explicit_close(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let events = Rc::new(RefCell::new(Vec::<PaneLifecycle>::new()));
    let capture = events.clone();
    let (_, visual) = cx.add_window_view(move |window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let workspace =
            cx.new(|cx| DockWorkspace::new("workspace-switch-regression", factory, window, cx));
        workspace.update(cx, |workspace, cx| {
            let lifecycle: LifecycleHook =
                Rc::new(move |event, _, _| capture.borrow_mut().push(event));
            workspace
                .add(
                    spec("chat:outgoing", lifecycle.clone(), cx),
                    DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            let original_panel = workspace.panels.borrow()["chat:outgoing"].entity_id();
            let original_content = workspace.panels.borrow()["chat:outgoing"]
                .read(cx)
                .spec
                .content
                .entity_id();
            let outgoing = workspace.serialize(cx).unwrap();
            workspace
                .register(spec("chat:incoming", lifecycle.clone(), cx), cx)
                .unwrap();
            let incoming_panel = workspace.panels.borrow()["chat:incoming"].entity_id();
            // Preserve the actual dock schema while making the selected layout distinct.
            let incoming: serde_json::Value = serde_json::from_str(
                &serde_json::to_string(&outgoing)
                    .unwrap()
                    .replace("chat:outgoing", "chat:incoming"),
            )
            .unwrap();
            workspace.restore_retaining(incoming, window, cx).unwrap();
            assert_eq!(
                DockWorkspace::saved_ids(&workspace.serialize(cx).unwrap()).unwrap(),
                vec!["chat:incoming"]
            );
            workspace.restore_retaining(outgoing, window, cx).unwrap();
            assert_eq!(
                DockWorkspace::saved_ids(&workspace.serialize(cx).unwrap()).unwrap(),
                vec!["chat:outgoing"]
            );
            assert_eq!(
                workspace.panels.borrow()["chat:outgoing"].entity_id(),
                original_panel
            );
            assert_eq!(
                workspace.panels.borrow()["chat:outgoing"]
                    .read(cx)
                    .spec
                    .content
                    .entity_id(),
                original_content
            );
            assert_eq!(
                workspace.panels.borrow()["chat:incoming"].entity_id(),
                incoming_panel
            );
            assert!(
                !events
                    .borrow()
                    .iter()
                    .any(|event| matches!(event, PaneLifecycle::Removed)),
                "switching must never signal closure"
            );
            assert!(
                events
                    .borrow()
                    .iter()
                    .any(|event| matches!(event, PaneLifecycle::Visible(false))),
                "outgoing pane must be hidden"
            );
            assert!(workspace.remove("chat:outgoing", window, cx));
            assert!(
                events
                    .borrow()
                    .iter()
                    .any(|event| matches!(event, PaneLifecycle::Removed)),
                "ordinary close must still signal closure"
            );
        });
        gpui_kit::component::Root::new(workspace, window, cx)
    });
    visual.run_until_parked();
}

#[gpui_kit::test]
fn workspace_restore_refocuses_last_browser_terminal_or_empty_pane(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| DockWorkspace::new("workspace-focus-test", factory, window, cx));
        dock.update(cx, |dock, cx| {
            let lifecycle: LifecycleHook = Rc::new(|_, _, _| {});
            for id in ["empty:focus", "browser:42", "terminal:focus"] {
                dock.add(spec(id, lifecycle.clone(), cx), DockPlacement::Center, None, window, cx).unwrap();
            }
            let identities = dock.panels.borrow().iter().map(|(id, panel)| (id.clone(), panel.entity_id())).collect::<Vec<_>>();
            let mut outgoing = dock.serialize(cx).unwrap();
            dock.register(spec("terminal:other-workspace", lifecycle.clone(), cx), cx).unwrap();
            let incoming = crate::workspace_tabs::empty_layout();
            for id in ["browser:42", "terminal:focus", "empty:focus"] {
                assert!(dock.focus(id, window, cx));
                let focused = dock.focused_pane_id(window, cx).unwrap();
                crate::workspace_visibility::remember_focus(&mut outgoing, Some(&focused));
                dock.restore_retaining(incoming.clone(), window, cx).unwrap();
                dock.restore_retaining(outgoing.clone(), window, cx).unwrap();
                for target in crate::workspace_visibility::focus_candidates(&outgoing, None) {
                    if dock.focus(&target, window, cx) { break; }
                }
                assert_eq!(dock.focused_pane_id(window, cx).as_deref(), Some(id));
                for (id, identity) in &identities { assert_eq!(dock.panels.borrow()[id].entity_id(), *identity); }
            }
        });
        gpui_kit::component::Root::new(dock, window, cx)
    });
    visual.run_until_parked();
}

#[gpui_kit::test]
fn sidebar_focus_restored_inactive_chat_and_closed_edge_retains_entities(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory_calls = Rc::new(RefCell::new(0));
        let calls = factory_calls.clone();
        let factory: PaneFactory = Rc::new(move |_, _, _| { *calls.borrow_mut() += 1; None });
        let dock = cx.new(|cx| DockWorkspace::new("sidebar-retained-focus", factory, window, cx));
        dock.update(cx, |dock, cx| {
            let lifecycle: LifecycleHook = Rc::new(|_, _, _| {});
            for id in ["chat:target", "chat:remembered"] {
                dock.add(spec(id, lifecycle.clone(), cx), DockPlacement::Center, None, window, cx).unwrap();
            }
            dock.add(spec("chat:edge", lifecycle, cx), DockPlacement::Left, None, window, cx).unwrap();
            let identities = dock.panels.borrow().iter().map(|(id, panel)|
                (id.clone(), panel.entity_id(), panel.read(cx).spec.content.entity_id())).collect::<Vec<_>>();
            dock.focus("chat:remembered", window, cx);
            let mut saved = dock.serialize(cx).unwrap();
            crate::workspace_visibility::remember_focus(&mut saved, Some("chat:remembered"));
            saved["left_dock"]["open"] = serde_json::Value::Bool(false);
            let empty = serde_json::json!({"version":1,"center":{"panel_name":"TabPanel",
                "children":[],"info":{"tabs":{"active_index":0}}}});
            dock.restore_retaining(empty, window, cx).unwrap();
            assert!(!dock.reveal_and_focus("chat:target", window, cx), "retained hidden pane is not current ownership");
            dock.restore_retaining(saved, window, cx).unwrap();
            for id in ["chat:target", "chat:target", "chat:edge"] {
                assert!(dock.reveal_and_focus(id, window, cx));
                assert_eq!(dock.focused_pane_id(window, cx).as_deref(), Some(id));
                assert_eq!(dock.panels.borrow().len(), identities.len(), "navigation never adds duplicate panes");
                for (id, panel, content) in &identities {
                    assert_eq!(dock.panels.borrow()[id].entity_id(), *panel);
                    assert_eq!(dock.panels.borrow()[id].read(cx).spec.content.entity_id(), *content);
                }
            }
            assert!(dock.area.read(cx).is_dock_open(DockPlacement::Left));
            assert_eq!(*factory_calls.borrow(), 0, "retained focus never recreates content");
        });
        gpui_kit::component::Root::new(dock, window, cx)
    });
    visual.run_until_parked();
}
