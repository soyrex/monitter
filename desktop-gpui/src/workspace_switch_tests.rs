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
fn workspace_switch_retains_pane_identity_until_explicit_close(cx: &mut TestAppContext) {
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
