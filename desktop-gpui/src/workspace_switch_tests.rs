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
