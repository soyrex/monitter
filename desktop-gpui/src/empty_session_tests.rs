//! A landing-tab selection reuses the retained chat and targets its own group.
use super::{DockWorkspace, PaneFactory, PaneSpec};
use gpui_kit::{AppContext as _, Context, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, Window, div, component::dock::DockPlacement};
use std::rc::Rc;

struct Content;
impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child("Existing chat")
    }
}

#[gpui_kit::test]
fn new_tab_session_selection_reuses_chat_in_destination_group(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| DockWorkspace::new("session-picker-test", factory, window, cx));
        workspace = Some(dock.clone());
        gpui_kit::component::Root::new(dock, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update(|window, cx| {
        let content = cx.new(|_| Content);
        let focus = cx.focus_handle();
        workspace.update(cx, |dock, cx| {
            dock.add(
                PaneSpec {
                    id: "chat:one".into(),
                    title: "One".into(),
                    content: content.into(),
                    focus,
                    lifecycle: None,
                    closable: true,
                },
                DockPlacement::Center,
                None,
                window,
                cx,
            )
            .unwrap();
            dock.focus("chat:one", window, cx);
            let original = dock.panel_key("chat:one").unwrap();
            let origin = dock.focused_group_node(window, cx).unwrap();
            let empty = dock.add_empty_split(origin, true, window, cx).unwrap();
            let empty_key = dock.panel_key(&empty).unwrap();
            let group = dock
                .area
                .read(cx)
                .layout(DockPlacement::Center)
                .unwrap()
                .find_panel_node(empty_key)
                .unwrap();
            assert!(dock.replace_empty_with_chat(&empty, "chat:one", window, cx));
            assert_eq!(dock.panel_key("chat:one"), Some(original));
            assert!(dock.panel_key(&empty).is_none());
            let area = dock.area.read(cx);
            let tree = area.layout(DockPlacement::Center).unwrap();
            assert_eq!(tree.find_panel_node(original), Some(group));
            assert_eq!(tree.panels().collect::<Vec<_>>(), vec![original]);
        });
    });
}
